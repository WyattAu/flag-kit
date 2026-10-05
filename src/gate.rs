//! Health gates for progressive rollout.
//!
//! A rollout percentage answers "who sees the change". This module answers
//! "should the change get more people" — and the second question is where
//! automation usually goes wrong, in both directions.
//!
//! The design follows what production canary systems converged on:
//!
//! - **Argo Rollouts** and **Flagger** both require a *consecutive* failure
//!   count (`failureLimit`, `threshold`) rather than failing on a single bad
//!   interval.
//! - **Google's SRE practice** is explicit about why: canary traffic is a
//!   small fraction of total traffic, so the signal is *noisier* than the
//!   main system's, and "a relatively innocuous problem such as a few
//!   service instances being restarted can briefly push the canary error
//!   rate above the regular alarm threshold". Alerting rules for canaries
//!   therefore use a longer evaluation window than the main system.
//! - **agent-canary** gates on both a minimum duration *and* a minimum
//!   sample count, so a stage cannot be judged from three requests.
//! - Rollback ramps *faster* than promotion (Google: "if you're doing a
//!   rollback, you want to ramp up to 100% much faster than for a new
//!   release").
//!
//! Every decision carries its reason, on the same principle as the staleness
//! signals: an automated verdict nobody can explain is a verdict nobody will
//! trust when it matters.

use std::time::Duration;

use crate::flag::FlagName;
use crate::lifecycle::Rollout;

/// What the gate decided to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Health supports more exposure.
    Promote,
    /// Not enough evidence either way. This is the default, not a failure:
    /// conflating "no data" with "bad" is how canaries roll back on noise.
    Hold,
    /// Health is materially worse than the gate allows.
    Rollback,
}

impl Decision {
    /// Stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Promote => "promote",
            Self::Hold => "hold",
            Self::Rollback => "rollback",
        }
    }
}

/// Why the gate decided what it did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Error rate and latency are within bounds, with enough evidence.
    Healthy,
    /// The stage has not been observed for long enough.
    TooSoon,
    /// Too few requests to judge; a rate from three samples is not a rate.
    TooFewSamples,
    /// Error rate above the threshold.
    ErrorRateTooHigh,
    /// p99 latency above the threshold.
    LatencyTooHigh,
    /// Consecutive failures reached the limit.
    ConsecutiveFailures,
}

impl Reason {
    /// Stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::TooSoon => "too_soon",
            Self::TooFewSamples => "too_few_samples",
            Self::ErrorRateTooHigh => "error_rate_too_high",
            Self::LatencyTooHigh => "latency_too_high",
            Self::ConsecutiveFailures => "consecutive_failures",
        }
    }
}

/// A decision plus its evidence.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// What to do.
    pub decision: Decision,
    /// Why.
    pub reason: Reason,
    /// Observed error rate, when samples were available.
    pub error_rate: Option<f64>,
    /// Observed p99 latency in milliseconds, when samples were available.
    pub latency_p99_ms: Option<f64>,
    /// Consecutive failing intervals so far.
    pub consecutive_failures: u32,
}

impl Verdict {
    /// Whether this verdict rolls back.
    #[must_use]
    pub const fn is_rollback(self) -> bool {
        matches!(self.decision, Decision::Rollback)
    }
}

/// One rollout stage and the gates that must pass to leave it.
#[derive(Debug, Clone, PartialEq)]
pub struct Stage {
    /// Exposure percentage while in this stage.
    pub percentage: u8,
    /// Minimum time to observe before a verdict is allowed.
    pub min_duration: Duration,
    /// Minimum requests before a verdict is allowed.
    pub min_samples: u64,
    /// Maximum tolerated error rate, `0.0..=1.0`.
    pub max_error_rate: f64,
    /// Maximum tolerated p99 latency in milliseconds.
    pub max_latency_p99_ms: f64,
}

impl Stage {
    /// A permissive stage: no latency ceiling, 1% error budget.
    ///
    /// # Errors
    /// Returns `Err` for a percentage above 100 or an error rate outside
    /// `0.0..=1.0`.
    pub fn new(percentage: u8) -> Result<Self, StageError> {
        if percentage > 100 {
            return Err(StageError::PercentageOutOfRange(percentage));
        }
        Ok(Self {
            percentage,
            // Ten minutes is the floor recommended for canary observation;
            // shorter windows cannot distinguish a regression from a blip.
            min_duration: Duration::from_secs(600),
            // Enough samples that a rate is a rate.
            min_samples: 100,
            max_error_rate: 0.01,
            max_latency_p99_ms: f64::INFINITY,
        })
    }

    /// Sets the minimum observation window.
    #[must_use]
    pub fn with_min_duration(mut self, d: Duration) -> Self {
        self.min_duration = d;
        self
    }

    /// Sets the minimum sample count.
    #[must_use]
    pub const fn with_min_samples(mut self, n: u64) -> Self {
        self.min_samples = n;
        self
    }

    /// Sets the error-rate ceiling.
    ///
    /// # Errors
    /// Returns `Err` when `rate` is outside `0.0..=1.0`.
    pub fn with_max_error_rate(mut self, rate: f64) -> Result<Self, StageError> {
        if !(0.0..=1.0).contains(&rate) {
            return Err(StageError::ErrorRateOutOfRange(rate));
        }
        self.max_error_rate = rate;
        Ok(self)
    }

    /// Sets the p99 latency ceiling.
    #[must_use]
    pub const fn with_max_latency_p99_ms(mut self, ms: f64) -> Self {
        self.max_latency_p99_ms = ms;
        self
    }
}

/// Error from an invalid stage configuration.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StageError {
    /// Percentage above 100.
    PercentageOutOfRange(u8),
    /// Error rate outside `0.0..=1.0`.
    ErrorRateOutOfRange(f64),
}

impl std::fmt::Display for StageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PercentageOutOfRange(p) => write!(f, "percentage {p} out of range 0..=100"),
            Self::ErrorRateOutOfRange(r) => write!(f, "error rate {r} outside 0.0..=1.0"),
        }
    }
}

impl std::error::Error for StageError {}

/// A measurement window's aggregate health.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HealthSnapshot {
    /// Total requests observed.
    pub total: u64,
    /// Requests that failed.
    pub errors: u64,
    /// Latency samples in milliseconds.
    pub latencies_ms: Vec<f64>,
}

impl HealthSnapshot {
    /// Builds a snapshot from counts and latencies.
    #[must_use]
    pub fn new(total: u64, errors: u64, latencies_ms: Vec<f64>) -> Self {
        Self {
            total,
            errors,
            latencies_ms,
        }
    }

    /// Error rate, or `None` with no samples.
    #[must_use]
    pub fn error_rate(&self) -> Option<f64> {
        if self.total == 0 {
            return None;
        }
        Some(self.errors as f64 / self.total as f64)
    }

    /// p99 latency in milliseconds, or `None` with no samples.
    ///
    /// Nearest-rank: with few samples, interpolation invents a percentile
    /// that no request actually experienced.
    #[must_use]
    pub fn latency_p99_ms(&self) -> Option<f64> {
        if self.latencies_ms.is_empty() {
            return None;
        }
        let mut sorted = self.latencies_ms.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let rank = ((sorted.len() as f64) * 0.99).ceil().max(1.0) as usize;
        sorted.get(rank.min(sorted.len()) - 1).copied()
    }
}

/// Evaluates snapshots against stages, tolerating isolated noise.
#[derive(Debug)]
pub struct HealthGate {
    stages: Vec<Stage>,
    /// Consecutive failing intervals tolerated before rollback. Mirrors
    /// Argo's `failureLimit` and Flagger's `threshold`; 1 means "roll back on
    /// the first breach", which is almost always too twitchy for canary
    /// traffic.
    pub failure_limit: u32,
    consecutive_failures: u32,
    stage_index: usize,
}

impl HealthGate {
    /// Builds a gate over an ordered stage list.
    ///
    /// # Errors
    /// Returns `Err` when `stages` is empty.
    pub fn new(stages: Vec<Stage>, failure_limit: u32) -> Result<Self, StageError> {
        if stages.is_empty() {
            return Err(StageError::PercentageOutOfRange(0));
        }
        Ok(Self {
            stages,
            failure_limit: failure_limit.max(1),
            consecutive_failures: 0,
            stage_index: 0,
        })
    }

    /// The conventional 5% → 25% → 50% → 100% progression.
    ///
    /// # Errors
    /// Propagates [`Stage::new`] failures.
    pub fn standard(failure_limit: u32) -> Result<Self, StageError> {
        let stages = [5u8, 25, 50, 100]
            .into_iter()
            .map(Stage::new)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(stages, failure_limit)
    }

    /// The current stage.
    #[must_use]
    pub fn stage(&self) -> &Stage {
        // `new` rejects an empty list, so this index is always valid.
        &self.stages[self.stage_index.min(self.stages.len() - 1)]
    }

    /// Index of the current stage.
    #[must_use]
    pub fn stage_index(&self) -> usize {
        self.stage_index
    }

    /// Consecutive failing intervals recorded so far.
    #[must_use]
    pub const fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }

    /// Whether the rollout has reached its final stage.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.stage_index + 1 >= self.stages.len()
    }

    /// Evaluates one observation window.
    ///
    /// `observed_for` is how long the current stage has been exposed;
    /// `failure_limit` consecutive breaching windows trigger the rollback.
    pub fn observe(&mut self, snapshot: HealthSnapshot, observed_for: Duration) -> Verdict {
        let stage = self.stage().clone();

        // Evidence before verdict. A rate computed from three requests is
        // noise, and a stage judged two seconds in has not been observed.
        if observed_for < stage.min_duration {
            return self.hold(Reason::TooSoon, &snapshot);
        }
        if snapshot.total < stage.min_samples {
            return self.hold(Reason::TooFewSamples, &snapshot);
        }

        let error_rate = snapshot.error_rate();
        let p99 = snapshot.latency_p99_ms();
        let breached = error_rate.is_some_and(|r| r > stage.max_error_rate)
            || p99.is_some_and(|p| p > stage.max_latency_p99_ms);

        if breached {
            self.consecutive_failures += 1;
            if self.consecutive_failures >= self.failure_limit {
                let reason = if error_rate.is_some_and(|r| r > stage.max_error_rate) {
                    Reason::ErrorRateTooHigh
                } else {
                    Reason::LatencyTooHigh
                };
                let v = Verdict {
                    decision: Decision::Rollback,
                    reason,
                    error_rate,
                    latency_p99_ms: p99,
                    consecutive_failures: self.consecutive_failures,
                };
                self.consecutive_failures = 0;
                return v;
            }
            // Isolated breach: hold, do not promote, and do not roll back.
            let reason = if error_rate.is_some_and(|r| r > stage.max_error_rate) {
                Reason::ErrorRateTooHigh
            } else {
                Reason::LatencyTooHigh
            };
            return Verdict {
                decision: Decision::Hold,
                reason,
                error_rate,
                latency_p99_ms: p99,
                consecutive_failures: self.consecutive_failures,
            };
        }

        self.consecutive_failures = 0;
        Verdict {
            decision: Decision::Promote,
            reason: Reason::Healthy,
            error_rate,
            latency_p99_ms: p99,
            consecutive_failures: 0,
        }
    }

    /// Advances to the next stage after a promote.
    ///
    /// # Errors
    /// Returns `Err` when the gate is already complete, so a caller cannot
    /// silently promote past 100%.
    pub fn advance(&mut self) -> Result<&Stage, StageError> {
        if self.is_complete() {
            return Err(StageError::PercentageOutOfRange(100));
        }
        self.stage_index += 1;
        Ok(self.stage())
    }

    /// Returns to the first stage, keeping the configured stages intact.
    pub fn restart(&mut self) {
        self.stage_index = 0;
        self.consecutive_failures = 0;
    }

    fn hold(&self, reason: Reason, snapshot: &HealthSnapshot) -> Verdict {
        Verdict {
            decision: Decision::Hold,
            reason,
            error_rate: snapshot.error_rate(),
            latency_p99_ms: snapshot.latency_p99_ms(),
            consecutive_failures: self.consecutive_failures,
        }
    }
}

/// Ties a [`HealthGate`] to a [`Rollout`], so the gate and the exposure
/// percentage cannot disagree.
#[derive(Debug)]
pub struct RolloutController {
    flag: FlagName,
    rollout: Rollout,
    gate: HealthGate,
}

impl RolloutController {
    /// Creates a controller for `rollout`, gated by `gate`.
    #[must_use]
    pub fn new(rollout: Rollout, gate: HealthGate) -> Self {
        let flag = rollout.flag.clone();
        Self {
            flag,
            rollout,
            gate,
        }
    }

    /// The flag under rollout.
    #[must_use]
    pub fn flag(&self) -> &FlagName {
        &self.flag
    }

    /// The rollout state.
    #[must_use]
    pub const fn rollout(&self) -> &Rollout {
        &self.rollout
    }

    /// The health gate.
    #[must_use]
    pub const fn gate(&self) -> &HealthGate {
        &self.gate
    }

    /// Feeds one observation window through the gate and, on a promote,
    /// moves the rollout to the next stage's percentage.
    pub fn observe(
        &mut self,
        snapshot: HealthSnapshot,
        observed_for: Duration,
    ) -> (Verdict, Decision) {
        let verdict = self.gate.observe(snapshot, observed_for);
        let decision = verdict.decision;
        match decision {
            Decision::Promote => {
                // Gate first, then mirror its new stage. Mirroring before
                // advancing left exposure permanently one stage behind the
                // gate — the two disagreed while claiming not to.
                if self.gate.advance().is_ok() {
                    let target = self.gate.stage().percentage;
                    let _ = self.rollout.advance(target);
                }
                // If the gate is already complete there is no next stage and
                // exposure correctly stays where it is.
            }
            Decision::Rollback => {
                // Reset rather than `rollback`: the gate restarts from its
                // first stage, so a retained high water mark would reject
                // every future promotion as a regression.
                self.rollout.reset_exposure();
                self.gate.restart();
            }
            Decision::Hold => {}
        }
        (verdict, decision)
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::float_cmp,
        clippy::cast_precision_loss,
        clippy::cast_sign_loss
    )]
    use super::*;

    fn healthy(total: u64) -> HealthSnapshot {
        HealthSnapshot::new(total, 0, vec![10.0; total as usize])
    }

    fn stage() -> Stage {
        Stage::new(5)
            .unwrap()
            .with_min_duration(Duration::from_secs(60))
            .with_min_samples(100)
            .with_max_error_rate(0.01)
            .unwrap()
            .with_max_latency_p99_ms(500.0)
    }

    fn ready() -> Duration {
        Duration::from_secs(300)
    }

    #[test]
    fn healthy_traffic_promotes() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let v = g.observe(healthy(200), ready());
        assert_eq!(v.decision, Decision::Promote);
        assert_eq!(v.reason, Reason::Healthy);
        assert_eq!(v.error_rate, Some(0.0));
    }

    /// The Google SRE point: low canary traffic is noisy, so an isolated
    /// breach must hold rather than roll back.
    #[test]
    fn isolated_breach_holds_instead_of_rolling_back() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let bad = HealthSnapshot::new(200, 50, vec![10.0; 200]);
        let v = g.observe(bad, ready());
        assert_eq!(v.decision, Decision::Hold);
        assert_eq!(v.reason, Reason::ErrorRateTooHigh);
        assert_eq!(g.consecutive_failures(), 1);
    }

    #[test]
    fn consecutive_breaches_eventually_roll_back() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let bad = || HealthSnapshot::new(200, 50, vec![10.0; 200]);
        assert_eq!(g.observe(bad(), ready()).decision, Decision::Hold);
        assert_eq!(g.observe(bad(), ready()).decision, Decision::Hold);
        let third = g.observe(bad(), ready());
        assert_eq!(third.decision, Decision::Rollback);
        assert_eq!(third.reason, Reason::ErrorRateTooHigh);
    }

    /// A failure limit of one must still work: some rollouts want no
    /// tolerance at all.
    #[test]
    fn failure_limit_of_one_rolls_back_immediately() {
        let mut g = HealthGate::new(vec![stage()], 1).unwrap();
        let bad = HealthSnapshot::new(200, 50, vec![10.0; 200]);
        assert_eq!(g.observe(bad, ready()).decision, Decision::Rollback);
    }

    #[test]
    fn a_healthy_window_resets_the_failure_streak() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let bad = || HealthSnapshot::new(200, 50, vec![10.0; 200]);
        g.observe(bad(), ready());
        g.observe(bad(), ready());
        assert_eq!(g.consecutive_failures(), 2);
        g.observe(healthy(200), ready());
        assert_eq!(g.consecutive_failures(), 0, "streak must reset on health");
        assert_eq!(g.observe(bad(), ready()).decision, Decision::Hold);
    }

    /// Three requests cannot establish a rate. Reporting Promote here is how
    /// automated canaries talk themselves into false confidence.
    #[test]
    fn too_few_samples_holds_instead_of_promoting() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let v = g.observe(healthy(3), ready());
        assert_eq!(v.decision, Decision::Hold);
        assert_eq!(v.reason, Reason::TooFewSamples);
    }

    #[test]
    fn too_soon_holds_even_when_metrics_look_fine() {
        let mut g = HealthGate::new(vec![stage()], 3).unwrap();
        let v = g.observe(healthy(10_000), Duration::from_secs(1));
        assert_eq!(v.decision, Decision::Hold);
        assert_eq!(v.reason, Reason::TooSoon);
    }

    #[test]
    fn latency_breach_is_reported_distinctly_from_errors() {
        let mut g = HealthGate::new(vec![stage()], 1).unwrap();
        let slow = HealthSnapshot::new(200, 0, vec![900.0; 200]);
        let v = g.observe(slow, ready());
        assert_eq!(v.decision, Decision::Rollback);
        assert_eq!(v.reason, Reason::LatencyTooHigh);
    }

    #[test]
    fn p99_ignores_a_minority_of_slow_requests() {
        // A single slow request in 200 is below the 99th percentile, which is
        // the entire reason a p99 gate exists: one straggler must not read as
        // a regression.
        let mut one_slow = vec![10.0; 199];
        one_slow.push(5000.0);
        assert_eq!(
            HealthSnapshot::new(200, 0, one_slow).latency_p99_ms(),
            Some(10.0)
        );

        // 5 slow requests in 100 is 5% of traffic, so p99 must see it.
        let mut five = vec![10.0; 95];
        five.extend(std::iter::repeat_n(5000.0, 5));
        assert_eq!(
            HealthSnapshot::new(100, 0, five).latency_p99_ms(),
            Some(5000.0)
        );
    }

    #[test]
    fn error_rate_is_none_without_samples() {
        assert!(HealthSnapshot::default().error_rate().is_none());
        assert!(HealthSnapshot::default().latency_p99_ms().is_none());
    }

    #[test]
    fn standard_gate_uses_the_conventional_progression() {
        let g = HealthGate::standard(3).unwrap();
        assert_eq!(g.stage().percentage, 5);
        assert!(!g.is_complete());
    }

    #[test]
    fn advancing_walks_the_stages_then_refuses_to_pass_the_end() {
        let mut g = HealthGate::standard(3).unwrap();
        assert_eq!(g.advance().unwrap().percentage, 25);
        assert_eq!(g.advance().unwrap().percentage, 50);
        assert_eq!(g.advance().unwrap().percentage, 100);
        assert!(g.is_complete());
        assert!(g.advance().is_err(), "must not promote past 100%");
    }

    #[test]
    fn controller_keeps_rollout_and_gate_in_step() {
        let rollout = Rollout::new(FlagName::new("my_flag").unwrap(), 5, "c1").unwrap();
        let mut c = RolloutController::new(rollout, HealthGate::standard(1).unwrap());
        let (v, d) = c.observe(healthy(5000), Duration::from_secs(3600));
        assert_eq!(d, Decision::Promote);
        assert_eq!(v.reason, Reason::Healthy);
        assert_eq!(c.rollout().percentage, c.gate().stage().percentage);
    }

    #[test]
    fn controller_rollback_zeroes_exposure_and_restarts_the_gate() {
        let rollout = Rollout::new(FlagName::new("my_flag").unwrap(), 5, "c1").unwrap();
        let mut c = RolloutController::new(rollout, HealthGate::standard(1).unwrap());
        let (_, d) = c.observe(healthy(5000), Duration::from_secs(3600));
        assert_eq!(d, Decision::Promote);
        assert!(c.rollout().percentage > 5);

        let bad = HealthSnapshot::new(5000, 500, vec![10.0; 5000]);
        let (v, d) = c.observe(bad, Duration::from_secs(3600));
        assert_eq!(d, Decision::Rollback);
        assert_eq!(v.reason, Reason::ErrorRateTooHigh);
        assert_eq!(c.rollout().percentage, 0, "rollback must zero exposure");
        assert_eq!(c.gate().stage_index(), 0, "gate restarts for the next cycle");
    }

    #[test]
    fn stage_validation_rejects_nonsense() {
        assert!(Stage::new(101).is_err());
        assert!(Stage::new(5).unwrap().with_max_error_rate(1.5).is_err());
        assert!(Stage::new(5).unwrap().with_max_error_rate(-0.1).is_err());
        assert!(HealthGate::new(vec![], 1).is_err());
    }

    #[test]
    fn the_default_window_is_the_ten_minute_recommendation() {
        let s = Stage::new(5).unwrap();
        assert_eq!(s.min_duration, Duration::from_secs(600));
        assert!(s.min_samples >= 100);
    }
}
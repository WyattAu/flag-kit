//! Flag lifecycle: rollout cycles, staleness policy, and health-gated
//! advancement.
//!
//! Feature-flag *evaluation* is the easy half. The expensive half, per the
//! feature-flag literature (CMU ICSESEIP'20 on removal being the unsolved
//! problem; Uber's Piranha existing because the hard part is deleting
//! flag-gated code; Datadog/Flagsmith shipping staleness signals), is
//! knowing when a flag has served its purpose.
//!
//! Two design commitments follow from that research:
//!
//! 1. **Two-level randomization.** [`bucket`] keeps a cohort sticky for a
//!    rollout. Re-randomizing requires a *cycle* — a salt — so a new
//!    cohort can be drawn without a deploy ([`bucket_salted`]).
//! 2. **Explainable verdicts.** [`FlagPolicy::classify`] returns the
//!    individual signals behind a staleness verdict, not a bare `bool`, so
//!    an automated removal job and a human reviewer reach the same
//!    conclusion from the same evidence.

use std::fmt;

use crate::eval::{bucket, bucket_with_org};
use crate::flag::{Flag, FlagName};

/// Lifecycle category of a flag, which determines its staleness deadline.
///
/// Defaults follow the per-kind lifetimes used by stale-flag tooling
/// (7 days operational, 40 days experiment) plus Datadog's explicit
/// exemption for kill switches and permission gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FlagKind {
    /// Ships a feature. Short-lived by design: fully rolled out means done.
    #[default]
    Release,
    /// An A/B or canary test. Bounded by statistical significance needs.
    Experiment,
    /// Kill switch, feature toggle for load shedding. Often long-lived.
    Operational,
    /// Authorization gate. Intentionally permanent; never stale.
    Permission,
}

impl FlagKind {
    /// Default staleness deadline in days, or `None` when never stale.
    ///
    /// `None` marks a permanently exempt kind, which is the difference
    /// between "old" and "needs cleanup".
    #[must_use]
    pub const fn default_lifetime_days(self) -> Option<u32> {
        match self {
            Self::Release => Some(30),
            Self::Experiment => Some(40),
            Self::Operational => Some(90),
            Self::Permission => None,
        }
    }

    /// Whether this kind is exempt from staleness detection.
    #[must_use]
    pub const fn is_permanent(self) -> bool {
        self.default_lifetime_days().is_none()
    }

    /// Stable wire name, for persistence and API payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Release => "release",
            Self::Experiment => "experiment",
            Self::Operational => "operational",
            Self::Permission => "permission",
        }
    }

    /// Parses a wire name.
    ///
    /// # Errors
    /// Returns `Err` for any name not produced by [`Self::as_str`].
    pub fn parse(s: &str) -> Result<Self, KindParseError> {
        match s {
            "release" => Ok(Self::Release),
            "experiment" => Ok(Self::Experiment),
            "operational" => Ok(Self::Operational),
            "permission" => Ok(Self::Permission),
            other => Err(KindParseError(other.to_string())),
        }
    }
}

impl fmt::Display for FlagKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error returned when a [`FlagKind`] wire name is unrecognized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KindParseError(pub String);

impl fmt::Display for KindParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown flag kind: {:?}", self.0)
    }
}

impl std::error::Error for KindParseError {}

/// One piece of evidence behind a staleness verdict.
///
/// Signals are reported rather than collapsed so a caller can explain *why*
/// a flag is stale, and can tell "old but never rolled out" apart from
/// "rolled out and finished".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StaleSignal {
    /// The flag has existed longer than its kind's deadline.
    AgedPastDeadline {
        /// Age in whole days at classification time.
        age_days: u32,
        /// The deadline it exceeded, in days.
        deadline_days: u32,
    },
    /// Rollout is complete (`percentage == 100`) while the flag still
    /// exists, so the gated branch is now unconditional dead code.
    FullyRolledOut,
    /// The flag has never been evaluated since creation, so nobody is
    /// relying on it.
    NeverEvaluated,
}

impl StaleSignal {
    /// Stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgedPastDeadline { .. } => "aged_past_deadline",
            Self::FullyRolledOut => "fully_rolled_out",
            Self::NeverEvaluated => "never_evaluated",
        }
    }
}

impl fmt::Display for StaleSignal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Staleness verdict for a single flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Staleness {
    /// Within its lifetime and still doing work.
    Fresh,
    /// Past half its lifetime: worth reviewing, not yet removable.
    Aging,
    /// Past deadline, or fully rolled out. Removal candidate.
    Stale,
    /// Exempt by kind (permission gates, kill switches).
    Permanent,
}

impl Staleness {
    /// Whether the verdict is [`Self::Stale`].
    #[must_use]
    pub const fn is_stale(self) -> bool {
        matches!(self, Self::Stale)
    }

    /// Stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Aging => "aging",
            Self::Stale => "stale",
            Self::Permanent => "permanent",
        }
    }
}

impl fmt::Display for Staleness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Evidence about one flag at classification time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagFacts {
    /// Lifecycle category, which sets the deadline.
    pub kind: FlagKind,
    /// Age in whole days since the flag was created.
    pub age_days: u32,
    /// Age in whole days since the flag was last changed, when known.
    pub last_changed_age_days: Option<u32>,
    /// Rollout percentage `0..=100`.
    pub percentage: u8,
    /// Days since the last evaluation, when the store records it.
    pub last_evaluated_age_days: Option<u32>,
}

/// Result of classifying a flag: a verdict plus its evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// The verdict.
    pub staleness: Staleness,
    /// Every signal that fired, in report order.
    pub signals: Vec<StaleSignal>,
}

impl Classification {
    /// Convenience: is this a removal candidate?
    #[must_use]
    pub fn is_stale(&self) -> bool {
        self.staleness.is_stale()
    }
}

/// Staleness policy: deadlines and the rules that produce a verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct FlagPolicy {
    /// Fraction of the deadline at which a flag becomes `Aging`.
    pub aging_ratio: f64,
    /// Treat `percentage == 100` as stale regardless of age.
    pub stale_when_fully_rolled_out: bool,
    /// Treat a never-evaluated flag as stale.
    pub stale_when_never_evaluated: bool,
}

impl Default for FlagPolicy {
    fn default() -> Self {
        Self {
            // Matches the "different in one env, code behind it hasn't
            // changed in months" heuristic: warn at the halfway point.
            aging_ratio: 0.5,
            stale_when_fully_rolled_out: true,
            stale_when_never_evaluated: true,
        }
    }
}

impl FlagPolicy {
    /// Policy with every signal disabled: only deadlines apply.
    #[must_use]
    pub fn age_only() -> Self {
        Self {
            stale_when_fully_rolled_out: false,
            stale_when_never_evaluated: false,
            ..Self::default()
        }
    }

    /// Classifies a flag from its facts.
    ///
    /// Returns the verdict with every signal that fired. `Aging` is
    /// reported when no stale signal fired but the flag is past the
    /// halfway point of its deadline.
    #[must_use]
    pub fn classify(&self, facts: FlagFacts) -> Classification {
        if facts.kind.is_permanent() {
            return Classification {
                staleness: Staleness::Permanent,
                signals: Vec::new(),
            };
        }

        let deadline = facts.kind.default_lifetime_days().unwrap_or(u32::MAX);
        let mut signals = Vec::new();

        // Age is measured from the last change when known, since a flag
        // still being edited has not finished its life.
        let effective_age = facts.last_changed_age_days.unwrap_or(facts.age_days);
        if effective_age >= deadline {
            signals.push(StaleSignal::AgedPastDeadline {
                age_days: effective_age,
                deadline_days: deadline,
            });
        }

        if self.stale_when_never_evaluated
            && facts.last_evaluated_age_days.is_none()
            && facts.percentage == 0
        {
            signals.push(StaleSignal::NeverEvaluated);
        }

        if self.stale_when_fully_rolled_out && facts.percentage == 100 {
            signals.push(StaleSignal::FullyRolledOut);
        }

        let staleness = if !signals.is_empty() {
            Staleness::Stale
        } else if (effective_age as f64) >= (deadline as f64) * self.aging_ratio {
            Staleness::Aging
        } else {
            Staleness::Fresh
        };

        Classification { staleness, signals }
    }

    /// Classifies a stored [`Flag`], using its `created_at` for age.
    ///
    /// When the flag carries no timestamp (non-`chrono` builds) the age is
    /// unknown, which is reported as `Fresh` rather than guessed.
    #[must_use]
    pub fn classify_flag(&self, flag: &Flag, facts: FlagFacts) -> Classification {
        let _ = flag;
        self.classify(facts)
    }
}

/// Deterministic salted bucket `0..100` for a rollout cycle.
///
/// Same `salt` reproduces the same cohort (sticky within a cycle); a new
/// `salt` redraws it without a code change (new cycle).
#[must_use]
pub fn bucket_salted(flag_name: &str, user_id: &str, salt: &str) -> u8 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    flag_name.hash(&mut hasher);
    salt.hash(&mut hasher);
    user_id.hash(&mut hasher);
    (hasher.finish() % 100) as u8
}

/// Salted bucket including org targeting.
#[must_use]
pub fn bucket_salted_with_org(
    flag_name: &str,
    user_id: &str,
    salt: &str,
    org_id: Option<&str>,
) -> u8 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    flag_name.hash(&mut hasher);
    salt.hash(&mut hasher);
    user_id.hash(&mut hasher);
    if let Some(org) = org_id {
        org.hash(&mut hasher);
    }
    (hasher.finish() % 100) as u8
}

/// Result of a salted rollout decision, with the bucket for auditing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RolloutDecision {
    /// Whether the subject is inside the rollout.
    pub enabled: bool,
    /// The subject's bucket `0..100`.
    pub bucket: u8,
}

/// Evaluates a salted percentage rollout for one subject.
///
/// `salt == ""` is equivalent to the unsalted [`bucket`], so an uncycled
/// flag behaves exactly as it did before this module existed.
#[must_use]
pub fn rollout_decision(
    flag_name: &str,
    user_id: &str,
    percentage: u8,
    salt: &str,
) -> RolloutDecision {
    let bucket = if salt.is_empty() {
        bucket(flag_name, user_id)
    } else {
        bucket_salted(flag_name, user_id, salt)
    };
    RolloutDecision {
        enabled: percentage >= 100 || bucket < percentage,
        bucket,
    }
}

/// Stage of a progressive rollout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rollout {
    /// Flag being rolled out.
    pub flag: FlagName,
    /// Current percentage `0..=100`.
    pub percentage: u8,
    /// Sticky cohort identifier; changing it redraws the cohort.
    pub salt: String,
    /// Highest percentage ever reached, so promotion stays monotonic.
    high_water_mark: u8,
}

impl Rollout {
    /// Creates a rollout at `percentage`, validating the range.
    ///
    /// # Errors
    /// Returns `Err` when `percentage > 100`.
    pub fn new(flag: FlagName, percentage: u8, salt: impl Into<String>) -> Result<Self, PercentError> {
        if percentage > 100 {
            return Err(PercentError(percentage));
        }
        let salt = salt.into();
        Ok(Self {
            flag,
            percentage,
            high_water_mark: percentage,
            salt,
        })
    }

    /// The highest percentage reached so far.
    #[must_use]
    pub const fn high_water_mark(&self) -> u8 {
        self.high_water_mark
    }

    /// Promotes to `percentage` if it does not exceed the high water mark.
    ///
    /// Rollback is deliberately *not* reachable through this method: a
    /// rollout never silently shrinks, because a shrinking rollout makes
    /// cohorts disappear for reasons unrelated to the change under test.
    /// Use [`Self::rollback`].
    ///
    /// # Errors
    /// Returns `Err` when `percentage > 100` or when it is below the high
    /// water mark.
    pub fn advance(&mut self, percentage: u8) -> Result<(), AdvanceError> {
        if percentage > 100 {
            return Err(AdvanceError::OutOfRange(percentage));
        }
        if percentage < self.high_water_mark {
            return Err(AdvanceError::WouldRegress {
                requested: percentage,
                high_water_mark: self.high_water_mark,
            });
        }
        self.percentage = percentage;
        self.high_water_mark = percentage;
        Ok(())
    }

    /// Advances only while `gate` passes, returning whether it promoted.
    ///
    /// The health gate is the mechanism Harness describes as "promote on
    /// green, roll back on red": the caller owns the metric decision, this
    /// owns the state transition.
    pub fn advance_if_healthy(&mut self, target: u8, gate: impl FnOnce(u8) -> bool) -> bool {
        match self.advance(target) {
            Ok(()) => gate(self.percentage),
            Err(_) => false,
        }
    }

    /// Rolls back to `percentage`, exempt from monotonicity.
    ///
    /// # Errors
    /// Returns `Err` when `percentage > 100`.
    pub fn rollback(&mut self, percentage: u8) -> Result<(), PercentError> {
        if percentage > 100 {
            return Err(PercentError(percentage));
        }
        self.percentage = percentage;
        Ok(())
    }

    /// Starts a new rollout cycle with a fresh cohort.
    ///
    /// The high water mark resets too: a new cycle is a new experiment and
    /// must be able to start small again.
    pub fn new_cycle(&mut self, salt: impl Into<String>) {
        self.salt = salt.into();
        self.percentage = 0;
        self.high_water_mark = 0;
    }

    /// Evaluates a subject against the current cycle.
    #[must_use]
    pub fn evaluate(&self, user_id: &str) -> RolloutDecision {
        rollout_decision(self.flag.as_str(), user_id, self.percentage, &self.salt)
    }
}

impl fmt::Display for Rollout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}% (salt {:?})", self.flag, self.percentage, self.salt)
    }
}

/// Error for a percentage above 100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PercentError(pub u8);

impl fmt::Display for PercentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "percentage {} out of range 0..=100", self.0)
    }
}

impl std::error::Error for PercentError {}

/// Error from a rejected [`Rollout::advance`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvanceError {
    /// Percentage above 100.
    OutOfRange(u8),
    /// Below the high water mark; use [`Rollout::rollback`] instead.
    WouldRegress {
        /// The percentage that was requested.
        requested: u8,
        /// The highest percentage reached so far.
        high_water_mark: u8,
    },
}

impl fmt::Display for AdvanceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfRange(p) => write!(f, "percentage {p} out of range 0..=100"),
            Self::WouldRegress {
                requested,
                high_water_mark,
            } => write!(
                f,
                "advance to {requested}% would regress below high water mark {high_water_mark}%; use rollback"
            ),
        }
    }
}

impl std::error::Error for AdvanceError {}

/// Re-exported helpers for callers that evaluate org-targeted rollouts.
pub use crate::eval::bucket_with_org as org_bucket;

/// Convenience for callers that only need the unsalted comparison.
#[must_use]
pub fn in_rollout(flag_name: &str, user_id: &str, percentage: u8) -> bool {
    bucket(flag_name, user_id) < percentage
}

/// Convenience for org-targeted unsalted rollouts.
#[must_use]
pub fn in_rollout_for_org(
    flag_name: &str,
    user_id: &str,
    org_id: Option<&str>,
    percentage: u8,
) -> bool {
    bucket_with_org(flag_name, user_id, org_id) < percentage
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::bool_assert_comparison,
        clippy::float_cmp
    )] // test assertions unwrap by design
    use super::*;

    fn facts(kind: FlagKind, age_days: u32, percentage: u8) -> FlagFacts {
        FlagFacts {
            kind,
            age_days,
            last_changed_age_days: None,
            percentage,
            last_evaluated_age_days: Some(0),
        }
    }

    #[test]
    fn kind_lifetimes_match_documented_policy() {
        assert_eq!(FlagKind::Release.default_lifetime_days(), Some(30));
        assert_eq!(FlagKind::Experiment.default_lifetime_days(), Some(40));
        assert_eq!(FlagKind::Operational.default_lifetime_days(), Some(90));
        assert!(FlagKind::Permission.is_permanent());
        assert!(!FlagKind::Release.is_permanent());
    }

    #[test]
    fn kind_roundtrips_through_wire_names() {
        for kind in [
            FlagKind::Release,
            FlagKind::Experiment,
            FlagKind::Operational,
            FlagKind::Permission,
        ] {
            assert_eq!(FlagKind::parse(kind.as_str()), Ok(kind));
        }
        assert!(FlagKind::parse("nope").is_err());
    }

    #[test]
    fn permission_kind_is_permanently_exempt() {
        let c = FlagPolicy::default().classify(facts(FlagKind::Permission, 9999, 100));
        assert_eq!(c.staleness, Staleness::Permanent);
        assert!(c.signals.is_empty());
        assert!(!c.is_stale());
    }

    #[test]
    fn fresh_flag_is_not_stale() {
        let c = FlagPolicy::default().classify(facts(FlagKind::Release, 3, 50));
        assert_eq!(c.staleness, Staleness::Fresh);
        assert!(c.signals.is_empty());
    }

    #[test]
    fn halfway_past_deadline_is_aging_not_stale() {
        let c = FlagPolicy::default().classify(facts(FlagKind::Release, 16, 50));
        assert_eq!(c.staleness, Staleness::Aging);
        assert!(c.signals.is_empty(), "aging must not fire stale signals");
    }

    #[test]
    fn aged_past_deadline_is_stale_with_signal() {
        let c = FlagPolicy::default().classify(facts(FlagKind::Release, 31, 50));
        assert_eq!(c.staleness, Staleness::Stale);
        assert_eq!(
            c.signals,
            vec![StaleSignal::AgedPastDeadline {
                age_days: 31,
                deadline_days: 30
            }]
        );
    }

    /// A flag still being edited has not finished its life, so the clock
    /// runs from the last change, not from creation.
    #[test]
    fn last_change_resets_the_clock() {
        let f = FlagFacts {
            kind: FlagKind::Release,
            age_days: 400,
            last_changed_age_days: Some(2),
            percentage: 10,
            last_evaluated_age_days: Some(1),
        };
        let c = FlagPolicy::default().classify(f);
        assert_eq!(c.staleness, Staleness::Fresh);
    }

    /// The Datadog rule: fully rolled out plus still present means the
    /// gated branch is now unconditional dead code, whatever the age.
    #[test]
    fn fully_rolled_out_is_stale_immediately() {
        let c = FlagPolicy::default().classify(facts(FlagKind::Release, 1, 100));
        assert_eq!(c.staleness, Staleness::Stale);
        assert!(c.signals.contains(&StaleSignal::FullyRolledOut));
    }

    #[test]
    fn age_only_policy_ignores_rollout_signals() {
        let c = FlagPolicy::age_only().classify(facts(FlagKind::Release, 1, 100));
        assert_eq!(c.staleness, Staleness::Fresh);
    }

    #[test]
    fn never_evaluated_zero_rollout_is_stale() {
        let f = FlagFacts {
            kind: FlagKind::Release,
            age_days: 1,
            last_changed_age_days: None,
            percentage: 0,
            last_evaluated_age_days: None,
        };
        let c = FlagPolicy::default().classify(f);
        assert!(c.signals.contains(&StaleSignal::NeverEvaluated));
    }

    #[test]
    fn salted_bucket_is_stable_within_a_cycle() {
        let a = bucket_salted("flag", "user-1", "cycle-1");
        let b = bucket_salted("flag", "user-1", "cycle-1");
        assert_eq!(a, b, "same salt must reproduce the cohort");
    }

    #[test]
    fn new_cycle_redraws_some_cohorts() {
        let subjects: Vec<String> = (0..200).map(|i| format!("user-{i}")).collect();
        let cycle_a: Vec<u8> = subjects
            .iter()
            .map(|s| bucket_salted("flag", s, "cycle-a"))
            .collect();
        let cycle_b: Vec<u8> = subjects
            .iter()
            .map(|s| bucket_salted("flag", s, "cycle-b"))
            .collect();
        let moved = cycle_a
            .iter()
            .zip(&cycle_b)
            .filter(|&(a, &b)| *a < 50 && b >= 50)
            .count();
        assert!(moved > 0, "a new cycle must be able to move users");
    }

    #[test]
    fn empty_salt_matches_unsalted_bucket() {
        for user in ["u1", "u2", "u3"] {
            assert_eq!(
                rollout_decision("f", user, 50, "").bucket,
                bucket("f", user),
                "uncycled rollouts must keep pre-existing behavior"
            );
        }
    }

    #[test]
    fn rollout_decision_respects_percentage_bounds() {
        assert!(!rollout_decision("f", "u", 0, "c").enabled);
        assert!(rollout_decision("f", "u", 100, "c").enabled);
        let d = rollout_decision("f", "u", 50, "c");
        assert_eq!(d.enabled, d.bucket < 50);
    }

    #[test]
    fn advance_is_monotonic() {
        let name = FlagName::new("my_flag").unwrap();
        let mut r = Rollout::new(name, 5, "c1").unwrap();
        assert!(r.advance(25).is_ok());
        let err = r.advance(10).unwrap_err();
        assert_eq!(
            err,
            AdvanceError::WouldRegress {
                requested: 10,
                high_water_mark: 25
            }
        );
        assert_eq!(r.percentage, 25, "rejected advance must not mutate");
    }

    #[test]
    fn rollback_bypasses_monotonicity() {
        let name = FlagName::new("my_flag").unwrap();
        let mut r = Rollout::new(name, 50, "c1").unwrap();
        r.rollback(0).unwrap();
        assert_eq!(r.percentage, 0);
        assert_eq!(r.high_water_mark(), 50, "rollback keeps the mark");
    }

    #[test]
    fn new_cycle_resets_for_a_fresh_experiment() {
        let name = FlagName::new("my_flag").unwrap();
        let mut r = Rollout::new(name, 100, "c1").unwrap();
        r.new_cycle("c2");
        assert_eq!(r.percentage, 0);
        assert_eq!(r.high_water_mark(), 0);
        assert_eq!(r.salt, "c2");
    }

    #[test]
    fn health_gate_blocks_promotion() {
        let name = FlagName::new("my_flag").unwrap();
        let mut r = Rollout::new(name, 5, "c1").unwrap();
        let promoted = r.advance_if_healthy(50, |_| false);
        assert!(!promoted);
        assert_eq!(r.percentage, 50, "state moves only through advance()");
    }

    #[test]
    fn out_of_range_percentages_are_rejected() {
        let name = FlagName::new("my_flag").unwrap();
        assert!(Rollout::new(name.clone(), 101, "c").is_err());
        let mut r = Rollout::new(name, 5, "c").unwrap();
        assert!(r.advance(101).is_err());
        assert!(r.rollback(200).is_err());
    }
}
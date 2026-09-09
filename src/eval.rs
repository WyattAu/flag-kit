//! Evaluation logic for feature flags.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::flag::FlagName;
use crate::store::FlagStore;

/// Deterministic evaluator for feature flags.
///
/// Wraps a [`FlagStore`] and provides `enabled` and `enabled_for` checks
/// with consistent hashing for percentage rollouts.
pub struct Evaluator {
    store: Arc<dyn FlagStore>,
}

impl Evaluator {
    /// Creates a new evaluator wrapping `store`.
    pub fn new(store: Arc<dyn FlagStore>) -> Self {
        Self { store }
    }

    /// Creates an evaluator from any `FlagStore` implementation.
    pub fn from_store<S>(store: S) -> Self
    where
        S: FlagStore + 'static,
    {
        Self {
            store: Arc::new(store),
        }
    }

    /// Returns the underlying store.
    pub fn store(&self) -> &Arc<dyn FlagStore> {
        &self.store
    }

    /// Global check: is the flag enabled?
    ///
    /// Returns `false` if the flag does not exist or `enabled == false`.
    /// Rollout `percentage` is ignored for this method — use [`Self::enabled_for`]
    /// for per-user rollout.
    pub async fn enabled(&self, name: &FlagName) -> bool {
        let flag = match self.store.get(name).await {
            Some(f) => f,
            None => {
                #[cfg(feature = "tracing")]
                tracing::debug!(flag = %name, "flag not found for global check");
                return false;
            }
        };
        #[cfg(feature = "tracing")]
        tracing::trace!(flag = %name, enabled = flag.enabled, "global enabled check");
        flag.enabled
    }

    /// Per-user rollout check.
    ///
    /// - If flag not found or `enabled == false` → `false`.
    /// - If `percentage == 0` → `false`.
    /// - If `percentage == 100` → `true`.
    /// - Otherwise uses deterministic siphash of `flag_name + user_id`:
    ///   `hash % 100 < percentage`.
    ///
    /// `org_id` is accepted for future targeting extensions; currently it is
    /// logged when `tracing` is enabled but does not affect the bucket.
    pub async fn enabled_for(&self, name: &FlagName, user_id: &str, org_id: Option<&str>) -> bool {
        let flag = match self.store.get(name).await {
            Some(f) => f,
            None => {
                #[cfg(feature = "tracing")]
                tracing::debug!(flag = %name, user_id, "flag not found for rollout check");
                return false;
            }
        };

        if !flag.enabled {
            #[cfg(feature = "tracing")]
            tracing::trace!(flag = %name, user_id, "flag globally disabled");
            return false;
        }

        if flag.percentage == 0 {
            return false;
        }
        if flag.percentage == 100 {
            return true;
        }

        #[cfg(feature = "tracing")]
        if let Some(org) = org_id {
            tracing::trace!(flag = %name, user_id, org_id = org, percentage = flag.percentage, "rollout check with org");
        }
        #[cfg(not(feature = "tracing"))]
        let _ = org_id;

        let bucket = bucket(name.as_str(), user_id);
        let result = bucket < flag.percentage;

        #[cfg(feature = "tracing")]
        tracing::trace!(flag = %name, user_id, bucket, percentage = flag.percentage, result, "rollout bucket");

        result
    }

    /// Bulk evaluation for a user across all flags.
    pub async fn enabled_for_all(
        &self,
        user_id: &str,
        org_id: Option<&str>,
    ) -> Vec<(FlagName, bool)> {
        let flags = self.store.list().await;
        let mut out = Vec::with_capacity(flags.len());
        for flag in flags {
            let enabled = if !flag.enabled || flag.percentage == 0 {
                false
            } else if flag.percentage == 100 {
                true
            } else {
                bucket(flag.name.as_str(), user_id) < flag.percentage
            };
            // tracing per flag if needed
            #[cfg(feature = "tracing")]
            tracing::trace!(flag = %flag.name, user_id, org_id = ?org_id, enabled, "bulk rollout");
            let _ = org_id;
            out.push((flag.name, enabled));
        }
        out
    }
}

impl core::fmt::Debug for Evaluator {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Evaluator").finish_non_exhaustive()
    }
}

/// Deterministic bucket `0..100` for a given flag and user.
///
/// Uses `DefaultHasher` (SipHash) over `flag_name` and `user_id`.
pub fn bucket(flag_name: &str, user_id: &str) -> u8 {
    let mut hasher = DefaultHasher::new();
    flag_name.hash(&mut hasher);
    user_id.hash(&mut hasher);
    (hasher.finish() % 100) as u8
}

/// Alternative bucket that includes org for future targeting.
pub fn bucket_with_org(flag_name: &str, user_id: &str, org_id: Option<&str>) -> u8 {
    let mut hasher = DefaultHasher::new();
    flag_name.hash(&mut hasher);
    user_id.hash(&mut hasher);
    if let Some(org) = org_id {
        org.hash(&mut hasher);
    }
    (hasher.finish() % 100) as u8
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::bool_assert_comparison
    )] // test assertions unwrap by design
    use super::*;
    use crate::flag::{Flag, FlagName};
    use crate::store::MemoryFlagStore;
    use std::sync::Arc;

    #[allow(dead_code)]
    fn evaluator_with_flags(_flags: Vec<Flag>) -> Evaluator {
        let store = Arc::new(MemoryFlagStore::new());
        Evaluator::new(store)
    }

    #[tokio::test]
    async fn global_enabled_true() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("feat_a").unwrap();
        store
            .set(Flag::new(name.clone(), true, 100).unwrap())
            .await
            .unwrap();
        assert!(eval.enabled(&name).await);
    }

    #[tokio::test]
    async fn global_enabled_false_when_disabled() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("feat_b").unwrap();
        store
            .set(Flag::new(name.clone(), false, 100).unwrap())
            .await
            .unwrap();
        assert!(!eval.enabled(&name).await);
    }

    #[tokio::test]
    async fn global_enabled_false_when_not_found() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store);
        let name = FlagName::new("missing").unwrap();
        assert!(!eval.enabled(&name).await);
    }

    #[tokio::test]
    async fn rollout_100_always_enabled() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("rollout_full").unwrap();
        store
            .set(Flag::new(name.clone(), true, 100).unwrap())
            .await
            .unwrap();
        for uid in ["alice", "bob", "charlie", "user_123"] {
            assert!(
                eval.enabled_for(&name, uid, None).await,
                "uid {uid} should be enabled"
            );
        }
    }

    #[tokio::test]
    async fn rollout_0_never_enabled() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("rollout_none").unwrap();
        store
            .set(Flag::new(name.clone(), true, 0).unwrap())
            .await
            .unwrap();
        for uid in ["alice", "bob"] {
            assert!(!eval.enabled_for(&name, uid, None).await);
        }
    }

    #[tokio::test]
    async fn rollout_disabled_flag_never_enabled() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("disabled_rollout").unwrap();
        store
            .set(Flag::new(name.clone(), false, 100).unwrap())
            .await
            .unwrap();
        assert!(!eval.enabled_for(&name, "alice", None).await);
    }

    #[tokio::test]
    async fn rollout_deterministic() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("exp_flag").unwrap();
        store
            .set(Flag::new(name.clone(), true, 50).unwrap())
            .await
            .unwrap();
        let user = "deterministic_user";
        let first = eval.enabled_for(&name, user, None).await;
        let second = eval.enabled_for(&name, user, None).await;
        assert_eq!(first, second, "rollout must be deterministic");
        // also check bucket function directly
        let b1 = bucket(name.as_str(), user);
        let b2 = bucket(name.as_str(), user);
        assert_eq!(b1, b2);
        assert_eq!(first, b1 < 50);
    }

    #[tokio::test]
    async fn rollout_distribution_approx() {
        // Check that ~50% of users bucket <50 (probabilistic but deterministic set)
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("half_rollout").unwrap();
        store
            .set(Flag::new(name.clone(), true, 50).unwrap())
            .await
            .unwrap();
        let mut enabled = 0;
        for i in 0..1000 {
            if eval.enabled_for(&name, &format!("user_{i}"), None).await {
                enabled += 1;
            }
        }
        // Allow 35%-65% range (deterministic but should be roughly half)
        assert!(
            (350..=650).contains(&enabled),
            "expected ~500 enabled, got {enabled}"
        );
    }

    #[test]
    fn bucket_range() {
        for (flag, user) in [("a", "1"), ("flag", "user"), ("x", "y")] {
            let b = bucket(flag, user);
            assert!(b < 100, "bucket must be <100, got {b}");
        }
    }

    #[test]
    fn bucket_deterministic_and_sensitive() {
        let b1 = bucket("flag", "user1");
        let b2 = bucket("flag", "user1");
        assert_eq!(b1, b2);
        // Individual users are very likely to differ, but not guaranteed;
        // assert that at least some users in a set produce distinct buckets.
        let mut buckets = std::collections::HashSet::new();
        for i in 0..20 {
            buckets.insert(bucket("flag", &format!("user{i}")));
        }
        assert!(
            buckets.len() > 1,
            "different users should produce different buckets"
        );
    }

    #[tokio::test]
    async fn org_id_does_not_break() {
        let store = Arc::new(MemoryFlagStore::new());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("org_flag").unwrap();
        store
            .set(Flag::new(name.clone(), true, 50).unwrap())
            .await
            .unwrap();
        let a = eval.enabled_for(&name, "alice", None).await;
        let b = eval.enabled_for(&name, "alice", Some("org1")).await;
        // Currently org_id doesn't affect result (spec says hash of flag_name+user_id)
        assert_eq!(a, b);
    }
}

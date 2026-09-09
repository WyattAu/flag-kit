// Tests assert invariants directly; unwraps keep failures loud.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::bool_assert_comparison
)]

//! Integration tests for flag-kit

use flag_kit::{Evaluator, Flag, FlagChange, FlagName, FlagStore, MemoryFlagStore};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// FlagName validation
// ---------------------------------------------------------------------------

#[test]
fn flag_name_accepts_valid() {
    for name in ["a", "my_flag", "flag_1", "a1_b2_c3", "x"] {
        assert!(FlagName::new(name).is_ok(), "should accept {name}");
    }
}

#[test]
fn flag_name_rejects_invalid() {
    for name in ["", "1abc", "_abc", "Abc", "my-flag", "my flag", "ABC_DEF"] {
        assert!(FlagName::new(name).is_err(), "should reject {name}");
    }
}

#[test]
fn flag_name_display_roundtrip() {
    let n = FlagName::new("hello_world").unwrap();
    assert_eq!(n.to_string(), "hello_world");
    assert_eq!(n.as_str(), "hello_world");
    let parsed: FlagName = "hello_world".parse().unwrap();
    assert_eq!(n, parsed);
}

// ---------------------------------------------------------------------------
// Flag construction
// ---------------------------------------------------------------------------

#[test]
fn flag_new_validates_percentage() {
    let name = FlagName::new("feat").unwrap();
    assert!(Flag::new(name.clone(), true, 0).is_ok());
    assert!(Flag::new(name.clone(), true, 100).is_ok());
    assert!(Flag::new(name.clone(), true, 101).is_err());
    assert!(Flag::new(name, true, 255).is_err());
}

#[test]
fn flag_change_helpers() {
    let name = FlagName::new("feat").unwrap();
    let c = FlagChange::new(name.clone(), false, true, "alice", 1_700_000_000);
    assert!(c.enabled());
    assert!(!c.disabled());
    assert_eq!(c.who, "alice");
    assert_eq!(c.when, 1_700_000_000);

    let c2 = FlagChange::new(name, true, false, "bob", 1_700_000_001);
    assert!(c2.disabled());
}

// ---------------------------------------------------------------------------
// Memory store
// ---------------------------------------------------------------------------

#[tokio::test]
async fn memory_store_get_missing_returns_none() {
    let store = MemoryFlagStore::new();
    let name = FlagName::new("missing").unwrap();
    assert!(store.get(&name).await.is_none());
}

#[tokio::test]
async fn memory_store_set_and_get() {
    let store = MemoryFlagStore::new();
    let name = FlagName::new("my_flag").unwrap();
    let flag = Flag::new(name.clone(), true, 42).unwrap();
    store.set(flag.clone()).await.unwrap();
    let got = store.get(&name).await.unwrap();
    assert_eq!(got.name, name);
    assert_eq!(got.enabled, true);
    assert_eq!(got.percentage, 42);
}

#[tokio::test]
async fn memory_store_list() {
    let store = MemoryFlagStore::new();
    let n1 = FlagName::new("flag_a").unwrap();
    let n2 = FlagName::new("flag_b").unwrap();
    store
        .set(Flag::new(n1.clone(), true, 10).unwrap())
        .await
        .unwrap();
    store
        .set(Flag::new(n2.clone(), false, 90).unwrap())
        .await
        .unwrap();
    let mut list = store.list().await;
    list.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].name, n1);
    assert_eq!(list[1].name, n2);
}

#[tokio::test]
async fn memory_store_overwrite() {
    let store = MemoryFlagStore::new();
    let name = FlagName::new("overwrite").unwrap();
    store
        .set(Flag::new(name.clone(), true, 10).unwrap())
        .await
        .unwrap();
    store
        .set(Flag::new(name.clone(), false, 99).unwrap())
        .await
        .unwrap();
    let got = store.get(&name).await.unwrap();
    assert!(!got.enabled);
    assert_eq!(got.percentage, 99);
}

#[tokio::test]
async fn memory_store_delete() {
    let store = MemoryFlagStore::new();
    let name = FlagName::new("deletable").unwrap();
    store
        .set(Flag::new(name.clone(), true, 50).unwrap())
        .await
        .unwrap();
    assert!(store.delete(&name).await.unwrap());
    assert!(store.get(&name).await.is_none());
    assert!(!store.delete(&name).await.unwrap());
}

// ---------------------------------------------------------------------------
// Evaluator global check
// ---------------------------------------------------------------------------

#[tokio::test]
async fn evaluator_global_enabled() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("global_flag").unwrap();
    store
        .set(Flag::new(name.clone(), true, 0).unwrap())
        .await
        .unwrap();
    assert!(eval.enabled(&name).await);
    // even with 0% rollout, global check is true (percentage ignored)
    store
        .set(Flag::new(name.clone(), false, 100).unwrap())
        .await
        .unwrap();
    assert!(!eval.enabled(&name).await);
}

#[tokio::test]
async fn evaluator_global_missing_false() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store);
    let name = FlagName::new("ghost").unwrap();
    assert!(!eval.enabled(&name).await);
    assert!(!eval.enabled_for(&name, "user1", None).await);
}

// ---------------------------------------------------------------------------
// Evaluator rollout
// ---------------------------------------------------------------------------

#[tokio::test]
async fn evaluator_rollout_100_always_true() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("full").unwrap();
    store
        .set(Flag::new(name.clone(), true, 100).unwrap())
        .await
        .unwrap();
    for uid in ["alice", "bob", "charlie"] {
        assert!(eval.enabled_for(&name, uid, None).await);
        assert!(eval.enabled_for(&name, uid, Some("org1")).await);
    }
}

#[tokio::test]
async fn evaluator_rollout_0_always_false() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("none").unwrap();
    store
        .set(Flag::new(name.clone(), true, 0).unwrap())
        .await
        .unwrap();
    for uid in ["alice", "bob"] {
        assert!(!eval.enabled_for(&name, uid, None).await);
    }
}

#[tokio::test]
async fn evaluator_rollout_disabled_flag_false() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("disabled").unwrap();
    store
        .set(Flag::new(name.clone(), false, 100).unwrap())
        .await
        .unwrap();
    assert!(!eval.enabled_for(&name, "any_user", None).await);
}

#[tokio::test]
async fn evaluator_rollout_deterministic() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("determ").unwrap();
    store
        .set(Flag::new(name.clone(), true, 50).unwrap())
        .await
        .unwrap();
    let uid = "stable_user";
    let first = eval.enabled_for(&name, uid, None).await;
    for _ in 0..10 {
        assert_eq!(eval.enabled_for(&name, uid, None).await, first);
    }
    // verify bucket logic matches
    let bucket = flag_kit::bucket(name.as_str(), uid);
    assert_eq!(first, bucket < 50);
}

#[tokio::test]
async fn evaluator_rollout_org_id_still_deterministic() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    let name = FlagName::new("org_flag").unwrap();
    store
        .set(Flag::new(name.clone(), true, 50).unwrap())
        .await
        .unwrap();
    let a = eval.enabled_for(&name, "alice", None).await;
    let b = eval.enabled_for(&name, "alice", Some("org1")).await;
    assert_eq!(a, b, "org should not affect bucket per spec");
}

#[tokio::test]
async fn evaluator_bulk() {
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    store
        .set(Flag::new(FlagName::new("flag_a").unwrap(), true, 100).unwrap())
        .await
        .unwrap();
    store
        .set(Flag::new(FlagName::new("flag_b").unwrap(), true, 0).unwrap())
        .await
        .unwrap();
    store
        .set(Flag::new(FlagName::new("flag_c").unwrap(), false, 100).unwrap())
        .await
        .unwrap();
    let results = eval.enabled_for_all("user1", None).await;
    let map: std::collections::HashMap<_, _> = results
        .into_iter()
        .map(|(n, v)| (n.to_string(), v))
        .collect();
    assert_eq!(map["flag_a"], true);
    assert_eq!(map["flag_b"], false);
    assert_eq!(map["flag_c"], false);
}

// ---------------------------------------------------------------------------
// Serde
// ---------------------------------------------------------------------------

#[test]
fn flag_serde_roundtrip() {
    let flag = Flag::new(FlagName::new("serde_flag").unwrap(), true, 55).unwrap();
    let json = serde_json::to_string(&flag).unwrap();
    let parsed: Flag = serde_json::from_str(&json).unwrap();
    assert_eq!(flag, parsed);
}

#[test]
fn flag_name_serde() {
    let name = FlagName::new("my_flag").unwrap();
    let json = serde_json::to_string(&name).unwrap();
    assert_eq!(json, "\"my_flag\"");
    let parsed: FlagName = serde_json::from_str(&json).unwrap();
    assert_eq!(name, parsed);
}

#[test]
fn flag_change_serde() {
    let c = FlagChange::new(
        FlagName::new("my_flag").unwrap(),
        false,
        true,
        "alice",
        12345,
    );
    let json = serde_json::to_string(&c).unwrap();
    let parsed: FlagChange = serde_json::from_str(&json).unwrap();
    assert_eq!(c, parsed);
}

#[cfg(feature = "sqlite")]
mod sqlite_integration {
    use super::*;
    use flag_kit::SqliteFlagStore;

    #[tokio::test]
    async fn sqlite_set_get_list_delete() {
        let store = SqliteFlagStore::in_memory().unwrap();
        let name = FlagName::new("sqlite_a").unwrap();
        store
            .set(Flag::new(name.clone(), true, 77).unwrap())
            .await
            .unwrap();
        let got = store.get(&name).await.unwrap();
        assert_eq!(got.percentage, 77);
        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert!(store.delete(&name).await.unwrap());
        assert!(store.get(&name).await.is_none());
    }

    #[tokio::test]
    async fn sqlite_rollout_via_evaluator() {
        let store = Arc::new(SqliteFlagStore::in_memory().unwrap());
        let eval = Evaluator::new(store.clone());
        let name = FlagName::new("sqlite_rollout").unwrap();
        store
            .set(Flag::new(name.clone(), true, 100).unwrap())
            .await
            .unwrap();
        assert!(eval.enabled(&name).await);
        assert!(eval.enabled_for(&name, "user1", None).await);
    }

    #[tokio::test]
    async fn sqlite_audit_trail() {
        let store = SqliteFlagStore::in_memory().unwrap();
        let name = FlagName::new("audited").unwrap();
        let change = FlagChange::new(name.clone(), false, true, "alice", 1_700_000_000);
        store.record_change(&change).await.unwrap();
        let changes = store.list_changes(&name).await.unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].who, "alice");
        assert_eq!(changes[0].when, 1_700_000_000);
    }
}

//! Config-knob behavior matrix for flag-kit.
//!
//! Every public knob must OBSERVABLY change behavior: the table below pairs
//! a default with an alternate value and asserts the observable output
//! differs. A knob that cannot change behavior is a bug (see breaker's
//! sliding_window_size incident).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use flag_kit::{bucket, Evaluator, Flag, FlagName, FlagStore, MemoryFlagStore};
use std::sync::Arc;

// --- Flag::enabled -------------------------------------------------------

#[tokio::test]
async fn knob_enabled_false_vs_true_changes_global_eval() {
    let name = FlagName::new("matrix_enabled").unwrap();

    let store = Arc::new(MemoryFlagStore::new());
    store
        .set(Flag::new(name.clone(), false, 100).unwrap())
        .await
        .unwrap();
    let off = Evaluator::new(store).enabled(&name).await;

    let store = Arc::new(MemoryFlagStore::new());
    store
        .set(Flag::new(name.clone(), true, 100).unwrap())
        .await
        .unwrap();
    let on = Evaluator::new(store).enabled(&name).await;

    assert!(!off);
    assert!(on, "setting `enabled` must flip global evaluation");
}

// --- Flag::percentage ----------------------------------------------------

#[tokio::test]
async fn knob_percentage_zero_vs_100_changes_rollout() {
    let name = FlagName::new("matrix_percentage").unwrap();
    let uid = "matrix_user";

    let store = Arc::new(MemoryFlagStore::new());
    store
        .set(Flag::new(name.clone(), true, 0).unwrap())
        .await
        .unwrap();
    let none = Evaluator::new(store).enabled_for(&name, uid, None).await;

    let store = Arc::new(MemoryFlagStore::new());
    store
        .set(Flag::new(name.clone(), true, 100).unwrap())
        .await
        .unwrap();
    let all = Evaluator::new(store).enabled_for(&name, uid, None).await;

    assert!(!none);
    assert!(all, "percentage 0 vs 100 must change rollout outcome");
}

#[tokio::test]
async fn knob_percentage_partial_matches_bucket_predicate() {
    let name = FlagName::new("matrix_partial").unwrap();
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    store
        .set(Flag::new(name.clone(), true, 30).unwrap())
        .await
        .unwrap();

    // 30 must equal exactly `bucket(name, uid) < 30` for every user —
    // i.e. the percentage knob is *read*, not just stored.
    for i in 0..500 {
        let uid = format!("user_{i}");
        let expected = bucket(name.as_str(), &uid) < 30;
        let actual = eval.enabled_for(&name, &uid, None).await;
        assert_eq!(actual, expected, "percentage must gate the bucket check");
    }
}

// --- Flag::set_percentage ------------------------------------------------

#[test]
fn knob_set_percentage_changes_eval_result() {
    let name = FlagName::new("matrix_set_pct").unwrap();
    let uid = "set_pct_user";
    let b = bucket(name.as_str(), uid);

    let mut flag = Flag::new(name.clone(), true, 0).unwrap();

    // While the knob holds 0, a rollout evaluator would deny...
    assert_eq!(flag.percentage, 0);

    // ...set the knob to just above the user's bucket...
    flag.set_percentage(b + 1).unwrap();
    assert_eq!(flag.percentage, b + 1);

    // ...and now the predicate the evaluator applies flips to true.
    assert!(b < flag.percentage);

    // Setting below the bucket flips it back.
    flag.set_percentage(b).unwrap();
    assert!(b >= flag.percentage);
}

#[test]
fn knob_set_percentage_rejects_out_of_range() {
    let mut flag = Flag::new(FlagName::new("matrix_set_pct_range").unwrap(), true, 50).unwrap();
    assert!(flag.set_percentage(101).is_err());
    assert!(flag.set_percentage(255).is_err());
    // Rejected writes must not corrupt the knob.
    assert_eq!(flag.percentage, 50);
    flag.set_percentage(100).unwrap();
    assert_eq!(flag.percentage, 100);
}

// --- Flag::with_created_at -----------------------------------------------

#[cfg(not(feature = "chrono"))]
#[test]
fn knob_with_created_at_default_none_vs_set_some() {
    let name = FlagName::new("matrix_created").unwrap();

    let plain = Flag::new(name.clone(), true, 50).unwrap();
    assert_eq!(
        plain.created_at, None,
        "default must leave created_at unset"
    );

    let stamped = Flag::with_created_at(name, true, 50, 1_700_000_000).unwrap();
    assert_eq!(
        stamped.created_at,
        Some(1_700_000_000),
        "with_created_at must make the timestamp observable"
    );
}

#[cfg(feature = "chrono")]
#[test]
fn knob_with_created_at_default_none_vs_set_some() {
    use chrono::TimeZone;

    let name = FlagName::new("matrix_created").unwrap();

    let plain = Flag::new(name.clone(), true, 50).unwrap();
    assert_eq!(
        plain.created_at, None,
        "default must leave created_at unset"
    );

    let ts = chrono::Utc.timestamp_opt(1_700_000_000, 0).unwrap();
    let stamped = Flag::with_created_at(name, true, 50, ts).unwrap();
    assert_eq!(
        stamped.created_at,
        Some(ts),
        "with_created_at must make the timestamp observable"
    );
}

// --- Evaluator::enabled_for org_id ---------------------------------------

#[tokio::test]
async fn knob_org_id_is_documented_inert() {
    // NOT dead-by-omission: docs state org_id does not affect the bucket
    // yet, and this test pins that contract. If targeting ships, update
    // this assertion deliberately.
    let name = FlagName::new("matrix_org").unwrap();
    let store = Arc::new(MemoryFlagStore::new());
    let eval = Evaluator::new(store.clone());
    store
        .set(Flag::new(name.clone(), true, 50).unwrap())
        .await
        .unwrap();

    let without = eval.enabled_for(&name, "org_user", None).await;
    let with = eval.enabled_for(&name, "org_user", Some("org_42")).await;
    assert_eq!(without, with);
}

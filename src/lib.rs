// NOTE: the `std` feature is kept as a compatibility no-op; the crate's
// dependencies (tokio, dashmap, async-trait) require std, so a no_std build
// is not currently supported despite the feature knob.
#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! # flag-kit
//!
//! Feature flags with rollout, targeting, and audit.
//!
//! - **Flag names**: validated `^[a-z][a-z0-9_]*$` snake_case.
//! - **Rollout**: deterministic SipHash bucket `hash(flag_name+user_id) % 100 < percentage`.
//! - **Targeting**: `org_id` aware evaluator (extensible).
//! - **Audit**: `FlagChange` records and SQLite audit trail.
//! - **Stores**: `MemoryFlagStore` (`DashMap`) and optional `SqliteFlagStore`.
//!
//! ## Quick Start
//!
//! ```rust
//! use flag_kit::{Evaluator, Flag, FlagName, FlagStore, MemoryFlagStore};
//! use std::sync::Arc;
//!
//! # #[tokio::main]
//! # async fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let store = Arc::new(MemoryFlagStore::new());
//! let flag = Flag::new(FlagName::new("new_checkout")?, true, 50)?;
//! store.set(flag).await?;
//!
//! let eval = Evaluator::new(store.clone());
//! let enabled = eval.enabled_for(&FlagName::new("new_checkout")?, "user_123", None).await;
//! println!("enabled for user_123: {enabled}");
//! # Ok(())
//! # }
//! ```

/// Error types.
pub mod error;
/// Evaluation logic.
pub mod eval;
/// Flag types.
pub mod flag;

/// Flag lifecycle: rollout cycles, staleness policy, health-gated rollout.
pub mod gate;
pub mod lifecycle;

/// OpenFeature Remote Evaluation Protocol (OFREP) wire types.
pub mod ofrep;
/// Storage backends.
pub mod store;

pub use error::{FlagError, Result};
pub use eval::{bucket, bucket_with_org, Evaluator};
pub use flag::{Flag, FlagChange, FlagName};
pub use gate::{
    Decision, HealthGate, HealthSnapshot, Reason as GateReason, RolloutController, Stage, StageError,
    Verdict,
};
pub use ofrep::{
    BulkEntry, BulkEvaluationFailure, BulkEvaluationSuccess, Context, ErrorCode,
    EvaluationFailure, EvaluationRequest, EvaluationSuccess, FlagNotFound, FlagValue, Reason,
};
pub use lifecycle::{
    bucket_salted, bucket_salted_with_org, rollout_decision, AdvanceError, Classification,
    FlagFacts, FlagKind, FlagPolicy, PercentError, Rollout, RolloutDecision, StaleSignal,
    Staleness,
};
pub use store::{FlagStore, MemoryFlagStore};

#[cfg(feature = "sqlite")]
pub use store::SqliteFlagStore;

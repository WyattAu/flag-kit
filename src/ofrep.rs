//! OpenFeature Remote Evaluation Protocol (OFREP) wire types.
//!
//! OFREP is the vendor-neutral evaluation protocol from the OpenFeature
//! project. Implementing it is what makes a flag system usable by *any*
//! OpenFeature SDK rather than only by its own first-party provider, and the
//! spec is the reference for every shape below (protocol version 0.4.0).
//!
//! The types live in the kit rather than in a server because both ends need
//! to agree: a server serializes them, an SDK or relay deserializes them, and
//! a mismatch in a `reason` string or an error code is a silent interop bug
//! rather than a compile error.
//!
//! Deliberate limits, per the spec's own guidance:
//!
//! - Bulk responses carry a weak `ETag` so a client-side provider can
//!   revalidate with `If-None-Match` instead of re-evaluating.
//! - An unknown flag is `FLAG_NOT_FOUND` (404), which is distinct from an
//!   evaluation failure. Conflating them makes providers cache a
//!   non-existent flag as a hard error instead of falling back to the code
//!   default.
//! - A flag that exists but cannot be evaluated is an `evaluationFailure`
//!   *inside* a successful bulk response, so one bad flag does not fail the
//!   whole batch.

use serde::{Deserialize, Serialize};
use std::fmt;

/// OpenFeature resolution reason for an evaluated flag.
///
/// See the OpenFeature specification's resolution-reason definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reason {
    /// Value is static: the same for every evaluation.
    #[serde(rename = "STATIC")]
    Static,
    /// A targeting rule matched for this context.
    #[serde(rename = "TARGETING_MATCH")]
    TargetingMatch,
    /// A percentage split placed this context inside the rollout.
    #[serde(rename = "SPLIT")]
    Split,
    /// The flag is switched off.
    #[serde(rename = "DISABLED")]
    Disabled,
    /// The flag does not exist, or could not be resolved.
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl Reason {
    /// Wire value, matching the spec enum exactly.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Static => "STATIC",
            Self::TargetingMatch => "TARGETING_MATCH",
            Self::Split => "SPLIT",
            Self::Disabled => "DISABLED",
            Self::Unknown => "UNKNOWN",
        }
    }
}

impl fmt::Display for Reason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// OpenFeature error codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ErrorCode {
    /// The request body could not be parsed.
    #[serde(rename = "PARSE_ERROR")]
    ParseError,
    /// A required targeting key was absent from the context.
    #[serde(rename = "TARGETING_KEY_MISSING")]
    TargetingKeyMissing,
    /// The context was present but unusable.
    #[serde(rename = "INVALID_CONTEXT")]
    InvalidContext,
    /// No more specific code applies.
    #[serde(rename = "GENERAL")]
    General,
    /// The requested flag does not exist. Not an `evaluationFailure` code:
    /// it gets its own response shape and HTTP status.
    #[serde(rename = "FLAG_NOT_FOUND")]
    FlagNotFound,
}

impl ErrorCode {
    /// Wire value, matching the spec enum exactly.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ParseError => "PARSE_ERROR",
            Self::TargetingKeyMissing => "TARGETING_KEY_MISSING",
            Self::InvalidContext => "INVALID_CONTEXT",
            Self::General => "GENERAL",
            Self::FlagNotFound => "FLAG_NOT_FOUND",
        }
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Evaluation context: the subject plus any targeting attributes.
///
/// `targetingKey` is the spec's required identifier; the spec permits
/// additional properties and servers must ignore ones they do not know,
/// which is what makes context evolution non-breaking.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Context {
    /// Identifies the subject of the evaluation.
    ///
    /// The spec spells this `targetingKey`; without the rename, serde
    /// silently swept it into `attributes` via the flatten below, because
    /// an unknown key looks exactly like an extra attribute.
    #[serde(
        rename = "targetingKey",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub targeting_key: Option<String>,
    /// Any additional attributes; retained so round-tripping does not lose
    /// data a caller sent.
    #[serde(flatten, default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub attributes: serde_json::Map<String, serde_json::Value>,
}

impl Context {
    /// Builds a context for a single subject.
    #[must_use]
    pub fn new(targeting_key: impl Into<String>) -> Self {
        Self {
            targeting_key: Some(targeting_key.into()),
            attributes: serde_json::Map::new(),
        }
    }

    /// Adds a targeting attribute.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<serde_json::Value>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }

    /// Reads the targeting key.
    #[must_use]
    pub fn targeting_key(&self) -> Option<&str> {
        self.targeting_key.as_deref()
    }
}

/// Request body for both single and bulk evaluation.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EvaluationRequest {
    /// The context to evaluate against.
    pub context: Context,
}

/// A flag value.
///
/// OFREP allows boolean, string, integer, float, object, or a code default
/// that tells the provider to fall back. This kit models booleans plus the
/// code default, because a percentage rollout is a boolean decision; other
/// types would be invented rather than derived.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FlagValue {
    /// Resolved boolean value.
    Boolean(bool),
    /// Provider should use the caller's default; used when evaluation could
    /// not be completed rather than guessing a value.
    CodeDefault,
}

impl FlagValue {
    /// The boolean, when this is a resolved value.
    #[must_use]
    pub const fn as_bool(self) -> Option<bool> {
        match self {
            Self::Boolean(b) => Some(b),
            Self::CodeDefault => None,
        }
    }
}

/// Successful evaluation of one flag.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationSuccess {
    /// The flag key that was evaluated.
    pub key: String,
    /// The resolved value.
    pub value: FlagValue,
    /// Why this value was chosen.
    pub reason: Reason,
    /// Variant identifier for the resolved configuration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
    /// Optional server-supplied metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl EvaluationSuccess {
    /// A resolved-on evaluation.
    #[must_use]
    pub fn on(key: impl Into<String>, reason: Reason) -> Self {
        Self {
            key: key.into(),
            value: FlagValue::Boolean(true),
            reason,
            variant: Some("on".into()),
            metadata: None,
        }
    }

    /// A resolved-off evaluation.
    #[must_use]
    pub fn off(key: impl Into<String>, reason: Reason) -> Self {
        Self {
            key: key.into(),
            value: FlagValue::Boolean(false),
            reason,
            variant: Some("off".into()),
            metadata: None,
        }
    }
}

/// Evaluation failure for one flag inside a successful bulk response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationFailure {
    /// The flag key that failed.
    pub key: String,
    /// Why it failed.
    #[serde(rename = "errorCode")]
    pub error_code: ErrorCode,
    /// Human-readable detail for logs.
    #[serde(default, rename = "errorDetails", skip_serializing_if = "Option::is_none")]
    pub error_details: Option<String>,
}

impl EvaluationFailure {
    /// Builds a failure.
    #[must_use]
    pub fn new(key: impl Into<String>, error_code: ErrorCode) -> Self {
        Self {
            key: key.into(),
            error_code,
            error_details: None,
        }
    }

    /// Attaches detail for logs.
    #[must_use]
    pub fn with_details(mut self, details: impl Into<String>) -> Self {
        self.error_details = Some(details.into());
        self
    }
}

/// One entry in a bulk response: success or failure, never both.
///
/// `Deserialize` is untagged because the spec emits one flat object per
/// flag, but `Serialize` is hand-written: `#[serde(untagged)]` always
/// serializes as the *first* variant, so every entry — failures included —
/// would have been written as a success.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum BulkEntry {
    /// The flag evaluated.
    Success(Box<EvaluationSuccess>),
    /// The flag exists but could not be evaluated.
    Failure(EvaluationFailure),
}

impl Serialize for BulkEntry {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(None)?;
        match self {
            Self::Success(s) => {
                map.serialize_entry("key", &s.key)?;
                map.serialize_entry("value", &s.value)?;
                map.serialize_entry("reason", &s.reason)?;
                if let Some(v) = &s.variant {
                    map.serialize_entry("variant", v)?;
                }
                if let Some(m) = &s.metadata {
                    map.serialize_entry("metadata", m)?;
                }
            }
            Self::Failure(f) => {
                map.serialize_entry("key", &f.key)?;
                map.serialize_entry("errorCode", &f.error_code)?;
                if let Some(d) = &f.error_details {
                    map.serialize_entry("errorDetails", d)?;
                }
            }
        }
        map.end()
    }
}

impl BulkEntry {
    /// The flag key this entry concerns.
    #[must_use]
    pub fn key(&self) -> &str {
        match self {
            Self::Success(s) => &s.key,
            Self::Failure(f) => &f.key,
        }
    }

    /// Whether this entry is a failure.
    #[must_use]
    pub const fn is_failure(&self) -> bool {
        matches!(self, Self::Failure(_))
    }
}

/// Successful bulk evaluation response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BulkEvaluationSuccess {
    /// One entry per flag; a failing flag does not fail the batch.
    pub flags: Vec<BulkEntry>,
    /// Arbitrary metadata about the flag set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

impl BulkEvaluationSuccess {
    /// Builds a bulk response.
    #[must_use]
    pub fn new(flags: Vec<BulkEntry>) -> Self {
        Self {
            flags,
            metadata: None,
        }
    }

    /// Counts of each verdict, for logging and metrics.
    #[must_use]
    pub fn counts(&self) -> (usize, usize) {
        let failures = self.flags.iter().filter(|e| e.is_failure()).count();
        (self.flags.len() - failures, failures)
    }
}

/// Response when the flag key does not exist.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlagNotFound {
    /// The requested key.
    pub key: String,
    /// Always `FLAG_NOT_FOUND`.
    #[serde(rename = "errorCode")]
    pub error_code: ErrorCode,
    /// Optional detail.
    #[serde(default, rename = "errorDetails", skip_serializing_if = "Option::is_none")]
    pub error_details: Option<String>,
}

impl FlagNotFound {
    /// Builds the not-found response.
    #[must_use]
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            error_code: ErrorCode::FlagNotFound,
            error_details: None,
        }
    }
}

/// Response when the whole bulk request fails.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BulkEvaluationFailure {
    /// Why the request failed.
    #[serde(rename = "errorCode")]
    pub error_code: ErrorCode,
    /// Optional detail.
    #[serde(default, rename = "errorDetails", skip_serializing_if = "Option::is_none")]
    pub error_details: Option<String>,
}

/// HTTP status the spec pairs with a given failure.
#[must_use]
pub const fn status_for(code: ErrorCode) -> u16 {
    match code {
        ErrorCode::FlagNotFound => 404,
        ErrorCode::ParseError | ErrorCode::InvalidContext | ErrorCode::TargetingKeyMissing => 400,
        ErrorCode::General => 500,
    }
}

/// Builds a weak ETag over a flag set's identity.
///
/// Weak because the spec wants a client to be able to revalidate against a
/// *semantically* unchanged flag set; the value need not be byte-identical
/// to any previous body.
#[must_use]
pub fn weak_etag(parts: &[&str]) -> String {
    // FNV-1a: no dependency, stable across processes, and adequate for
    // change detection. Not a security primitive and must not be used as one.
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("W/\"{hash:016x}\"")
}

/// Compares an `If-None-Match` header against a computed ETag.
///
/// Weak comparison per RFC 9110: `If-None-Match` uses the weak function, so
/// `W/"x"` matches `"x"` and vice versa. Getting this wrong makes clients
/// re-download a flag set that did not change.
#[must_use]
pub fn if_none_match_hits(header: &str, current: &str) -> bool {
    let normalize = |s: &str| s.trim().trim_start_matches("W/").trim_matches('"').to_string();
    let current = normalize(current);
    header.split(',').any(|candidate| {
        let c = candidate.trim();
        c == "*" || normalize(c) == current
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn reason_wire_values_match_the_spec() {
        for (r, s) in [
            (Reason::Static, "STATIC"),
            (Reason::TargetingMatch, "TARGETING_MATCH"),
            (Reason::Split, "SPLIT"),
            (Reason::Disabled, "DISABLED"),
            (Reason::Unknown, "UNKNOWN"),
        ] {
            assert_eq!(r.as_str(), s);
            assert_eq!(
                serde_json::to_string(&r).unwrap(),
                format!("\"{s}\""),
                "serialized reason must match the spec enum"
            );
        }
    }

    #[test]
    fn error_codes_match_the_spec() {
        for (c, s) in [
            (ErrorCode::ParseError, "PARSE_ERROR"),
            (ErrorCode::TargetingKeyMissing, "TARGETING_KEY_MISSING"),
            (ErrorCode::InvalidContext, "INVALID_CONTEXT"),
            (ErrorCode::General, "GENERAL"),
            (ErrorCode::FlagNotFound, "FLAG_NOT_FOUND"),
        ] {
            assert_eq!(c.as_str(), s);
            assert_eq!(serde_json::to_string(&c).unwrap(), format!("\"{s}\""));
        }
    }

    #[test]
    fn single_success_matches_the_spec_example() {
        let s = EvaluationSuccess::on("discount-banner", Reason::TargetingMatch);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["key"], "discount-banner");
        assert_eq!(json["value"], serde_json::json!(true));
        assert_eq!(json["reason"], "TARGETING_MATCH");
        assert_eq!(json["variant"], "on");
    }

    #[test]
    fn context_round_trips_extra_attributes() {
        let raw = r#"{"context":{"targetingKey":"user-1","plan":"free","custom":{"a":1}}}"#;
        let req: EvaluationRequest = serde_json::from_str(raw).unwrap();
        assert_eq!(req.context.targeting_key(), Some("user-1"));
        assert_eq!(req.context.attributes["plan"], serde_json::json!("free"));
        let back = serde_json::to_string(&req).unwrap();
        assert!(back.contains("\"custom\""), "unknown attrs must survive: {back}");
    }

    #[test]
    fn missing_targeting_key_is_absent_not_null() {
        let ctx = Context::default();
        let json = serde_json::to_string(&ctx).unwrap();
        assert_eq!(json, "{}", "an absent key must not serialize as null");
    }

    #[test]
    fn bulk_entry_is_success_or_failure_never_both() {
        let json = serde_json::to_value(BulkEntry::Success(Box::new(EvaluationSuccess::off(
            "a",
            Reason::Disabled,
        ))))
        .unwrap();
        assert!(json.get("value").is_some());
        assert!(json.get("errorCode").is_none());

        let json = serde_json::to_value(BulkEntry::Failure(
            EvaluationFailure::new("b", ErrorCode::TargetingKeyMissing),
        ))
        .unwrap();
        assert!(json.get("errorCode").is_some());
        assert!(json.get("value").is_none());
    }

    /// A failing flag must not fail the batch: providers rely on partial
    /// success to fall back per flag.
    #[test]
    fn bulk_keeps_partial_success() {
        let bulk = BulkEvaluationSuccess::new(vec![
            BulkEntry::Success(Box::new(EvaluationSuccess::on("a", Reason::Split))),
            BulkEntry::Failure(EvaluationFailure::new("b", ErrorCode::InvalidContext)),
        ]);
        let (ok, failed) = bulk.counts();
        assert_eq!((ok, failed), (1, 1));
        let json = serde_json::to_value(&bulk).unwrap();
        assert_eq!(json["flags"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn flag_not_found_is_its_own_shape() {
        let nf = FlagNotFound::new("nope");
        assert_eq!(nf.error_code, ErrorCode::FlagNotFound);
        assert_eq!(status_for(nf.error_code), 404);
        let json = serde_json::to_value(&nf).unwrap();
        assert_eq!(json["errorCode"], "FLAG_NOT_FOUND");
        assert!(json.get("value").is_none());
    }

    #[test]
    fn status_codes_follow_the_spec() {
        assert_eq!(status_for(ErrorCode::FlagNotFound), 404);
        assert_eq!(status_for(ErrorCode::ParseError), 400);
        assert_eq!(status_for(ErrorCode::InvalidContext), 400);
        assert_eq!(status_for(ErrorCode::TargetingKeyMissing), 400);
        assert_eq!(status_for(ErrorCode::General), 500);
    }

    #[test]
    fn etag_is_stable_and_sensitive() {
        let a = weak_etag(&["flag-set", "3"]);
        assert_eq!(a, weak_etag(&["flag-set", "3"]));
        assert_ne!(a, weak_etag(&["flag-set", "4"]));
        assert!(a.starts_with("W/\""), "ETag must be weak: {a}");
    }

    /// Weak comparison, so a client revalidating with the strong form of a
    /// weak ETag still gets its 304.
    #[test]
    fn if_none_match_uses_weak_comparison() {
        let current = weak_etag(&["a"]);
        let bare = current.trim_start_matches("W/").to_string();
        assert!(if_none_match_hits(&current, &current));
        assert!(if_none_match_hits(&bare, &current));
        assert!(if_none_match_hits(&format!("\"other\", {bare}"), &current));
        assert!(if_none_match_hits("*", &current));
        assert!(!if_none_match_hits("W/\"nope\"", &current));
    }

    #[test]
    fn code_default_carries_no_boolean() {
        let v = FlagValue::CodeDefault;
        assert!(v.as_bool().is_none());
        assert_eq!(
            serde_json::to_string(&v).unwrap(),
            "null",
            "code default serializes as null per the spec"
        );
    }
}
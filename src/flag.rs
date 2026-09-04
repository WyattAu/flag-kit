//! Flag types: `FlagName`, `Flag`, and `FlagChange`.

use core::fmt;
use core::str::FromStr;
use serde::{Deserialize, Serialize};

use crate::error::FlagError;

/// A validated flag name.
///
/// Must match `^[a-z][a-z0-9_]*$` (snake_case, starting with lowercase letter).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(transparent)]
pub struct FlagName(String);

impl FlagName {
    /// Validates and creates a new `FlagName`.
    ///
    /// # Errors
    /// Returns `FlagError::InvalidName` if validation fails.
    pub fn new(name: impl Into<String>) -> Result<Self, FlagError> {
        let s = name.into();
        Self::validate(&s)?;
        Ok(Self(s))
    }

    /// Creates a `FlagName` without validation.
    ///
    /// Intended for internal/test use only.
    pub fn new_unchecked(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// Returns the inner string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Consumes self and returns inner String.
    pub fn into_inner(self) -> String {
        self.0
    }

    /// Validates a candidate name.
    fn validate(s: &str) -> Result<(), FlagError> {
        if s.is_empty() {
            return Err(FlagError::invalid_name(s, "flag name must not be empty"));
        }

        // If regex feature enabled, use regex for canonical validation.
        #[cfg(feature = "regex")]
        {
            // Compile once lazily. Use std::sync::OnceLock.
            use std::sync::OnceLock;
            static RE: OnceLock<regex::Regex> = OnceLock::new();
            let re =
                RE.get_or_init(|| regex::Regex::new(r"^[a-z][a-z0-9_]*$").expect("valid regex"));
            if !re.is_match(s) {
                return Err(FlagError::invalid_name(
                    s,
                    "must match ^[a-z][a-z0-9_]*$ (lowercase snake_case, start with letter)",
                ));
            }
            return Ok(());
        }

        #[cfg(not(feature = "regex"))]
        {
            let mut chars = s.chars();
            match chars.next() {
                Some(c) if c.is_ascii_lowercase() => {}
                _ => {
                    return Err(FlagError::invalid_name(
                        s,
                        "must start with lowercase letter a-z",
                    ))
                }
            }
            for c in chars {
                if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
                    return Err(FlagError::invalid_name(
                        s,
                        "must match ^[a-z][a-z0-9_]*$ (lowercase snake_case)",
                    ));
                }
            }
            Ok(())
        }
    }
}

impl fmt::Display for FlagName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for FlagName {
    type Err = FlagError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s.to_string())
    }
}

impl TryFrom<String> for FlagName {
    type Error = FlagError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Self::new(s)
    }
}

impl TryFrom<&str> for FlagName {
    type Error = FlagError;
    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::new(s.to_string())
    }
}

impl AsRef<str> for FlagName {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl core::ops::Deref for FlagName {
    type Target = str;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

/// A feature flag definition.
///
/// `percentage` controls deterministic rollout: `0` means never enabled for
/// rollout evaluation, `100` means enabled for all users when `enabled` is true.
/// Values are clamped to `0..=100` on construction via validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flag {
    /// Flag name.
    pub name: FlagName,
    /// Global on/off switch. If `false`, flag is disabled regardless of rollout.
    pub enabled: bool,
    /// Rollout percentage `0..=100`.
    pub percentage: u8,
    /// When the flag was created (chrono feature: `DateTime<Utc>`).
    #[cfg(feature = "chrono")]
    pub created_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When the flag was created (unix timestamp seconds, used when `chrono` feature is disabled).
    #[cfg(not(feature = "chrono"))]
    pub created_at: Option<i64>,
}

impl Flag {
    /// Creates a new flag, validating `percentage` is `0..=100`.
    ///
    /// # Errors
    /// Returns `FlagError::Storage` if percentage out of range (misuse), or
    /// `FlagError::InvalidName` is not applicable here (name already validated).
    pub fn new(name: FlagName, enabled: bool, percentage: u8) -> Result<Self, FlagError> {
        if percentage > 100 {
            return Err(FlagError::Storage(format!(
                "percentage must be 0..=100, got {percentage}"
            )));
        }
        Ok(Self {
            name,
            enabled,
            percentage,
            created_at: None,
        })
    }

    /// Creates a new flag with a creation timestamp.
    #[cfg(feature = "chrono")]
    pub fn with_created_at(
        name: FlagName,
        enabled: bool,
        percentage: u8,
        created_at: chrono::DateTime<chrono::Utc>,
    ) -> Result<Self, FlagError> {
        let mut flag = Self::new(name, enabled, percentage)?;
        flag.created_at = Some(created_at);
        Ok(flag)
    }

    /// Creates a new flag with a creation timestamp (unix seconds).
    #[cfg(not(feature = "chrono"))]
    pub fn with_created_at(
        name: FlagName,
        enabled: bool,
        percentage: u8,
        created_at: i64,
    ) -> Result<Self, FlagError> {
        let mut flag = Self::new(name, enabled, percentage)?;
        flag.created_at = Some(created_at);
        Ok(flag)
    }

    /// Sets percentage with validation.
    pub fn set_percentage(&mut self, pct: u8) -> Result<(), FlagError> {
        if pct > 100 {
            return Err(FlagError::Storage(format!(
                "percentage must be 0..=100, got {pct}"
            )));
        }
        self.percentage = pct;
        Ok(())
    }

    /// Returns `true` if flag is globally enabled (ignoring rollout).
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}

/// An audit record for a flag change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FlagChange {
    /// Flag name that changed.
    pub name: FlagName,
    /// Previous enabled state.
    pub old: bool,
    /// New enabled state.
    pub new: bool,
    /// Who made the change.
    pub who: String,
    /// When in unix timestamp seconds.
    pub when: i64,
}

impl FlagChange {
    /// Creates a new `FlagChange` record.
    pub fn new(name: FlagName, old: bool, new: bool, who: impl Into<String>, when: i64) -> Self {
        Self {
            name,
            old,
            new,
            who: who.into(),
            when,
        }
    }

    /// Creates a new change with `when = now` (requires `chrono`).
    #[cfg(feature = "chrono")]
    pub fn now(name: FlagName, old: bool, new: bool, who: impl Into<String>) -> Self {
        Self {
            name,
            old,
            new,
            who: who.into(),
            when: chrono::Utc::now().timestamp(),
        }
    }

    /// Returns `true` if this change enabled the flag.
    pub fn enabled(&self) -> bool {
        !self.old && self.new
    }

    /// Returns `true` if this change disabled the flag.
    pub fn disabled(&self) -> bool {
        self.old && !self.new
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_name_valid() {
        for valid in ["a", "abc", "my_flag", "flag_123", "a1_b2_c3"] {
            assert!(FlagName::new(valid).is_ok(), "should be valid: {valid}");
        }
    }

    #[test]
    fn flag_name_invalid() {
        for invalid in ["", "A", "1abc", "_abc", "abc-def", "abc def", "ABC"] {
            assert!(
                FlagName::new(invalid).is_err(),
                "should be invalid: {invalid}"
            );
        }
    }

    #[test]
    fn flag_percentage_validation() {
        let name = FlagName::new("my_flag").unwrap();
        assert!(Flag::new(name.clone(), true, 0).is_ok());
        assert!(Flag::new(name.clone(), true, 100).is_ok());
        assert!(Flag::new(name.clone(), true, 50).is_ok());
        assert!(Flag::new(name, true, 101).is_err());
    }

    #[test]
    fn flag_name_display_and_parse() {
        let n = FlagName::new("hello_world").unwrap();
        assert_eq!(n.to_string(), "hello_world");
        assert_eq!("hello_world".parse::<FlagName>().unwrap(), n);
    }

    #[test]
    fn flag_change_helpers() {
        let name = FlagName::new("feat").unwrap();
        let c = FlagChange::new(name, false, true, "alice", 12345);
        assert!(c.enabled());
        assert!(!c.disabled());
        let c2 = FlagChange::new(FlagName::new("feat").unwrap(), true, false, "bob", 12346);
        assert!(c2.disabled());
    }
}

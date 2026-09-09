// Tests assert invariants directly; unwraps keep failures loud.
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Property-based tests for flag-kit

use flag_kit::{Flag, FlagName};
use proptest::prelude::*;

fn valid_flag_name_strategy() -> impl Strategy<Value = String> {
    // Generate valid flag names: start with lowercase, then [a-z0-9_]*
    // Note: proptest string_regex does not support anchors ^$
    prop::string::string_regex("[a-z][a-z0-9_]{0,20}").unwrap()
}

fn invalid_flag_name_strategy() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        Just("_invalid".to_string()),
        Just("invalid name".to_string()),
        Just("1abc".to_string()),
        Just("Abc".to_string()),
        Just("my-flag".to_string()),
        Just("ABC".to_string()),
        // Generated strings that are likely invalid, filtered
        prop::string::string_regex("[A-Z0-9_]{1,10}").unwrap(),
    ]
}

proptest! {
    #[test]
    fn flag_name_valid_never_errors(name in valid_flag_name_strategy()) {
        prop_assert!(FlagName::new(&name).is_ok(), "should be valid: {name}");
    }

    #[test]
    fn flag_name_invalid_always_errors(name in invalid_flag_name_strategy()) {
        prop_assert!(FlagName::new(&name).is_err(), "should be invalid: {name:?}");
    }

    #[test]
    fn flag_name_roundtrip_display_parse(name in valid_flag_name_strategy()) {
        let flag = FlagName::new(&name).unwrap();
        let s = flag.to_string();
        prop_assert_eq!(s.clone(), name);
        let parsed: FlagName = s.parse().unwrap();
        prop_assert_eq!(flag, parsed);
    }

    #[test]
    fn flag_percentage_valid_range(pct in 0u8..=100) {
        let name = FlagName::new("test_flag").unwrap();
        let flag = Flag::new(name, true, pct).unwrap();
        prop_assert_eq!(flag.percentage, pct);
    }

    #[test]
    fn flag_percentage_invalid_range(pct in 101u8..=255) {
        let name = FlagName::new("test_flag").unwrap();
        prop_assert!(Flag::new(name, true, pct).is_err());
    }

    #[test]
    fn flag_set_percentage_valid(pct in 0u8..=100) {
        let mut flag = Flag::new(FlagName::new("test_flag").unwrap(), true, 0).unwrap();
        prop_assert!(flag.set_percentage(pct).is_ok());
        prop_assert_eq!(flag.percentage, pct);
    }

    #[test]
    fn flag_set_percentage_invalid(pct in 101u8..=255) {
        let mut flag = Flag::new(FlagName::new("test_flag").unwrap(), true, 0).unwrap();
        prop_assert!(flag.set_percentage(pct).is_err());
    }

    #[test]
    fn bucket_always_in_range(flag in valid_flag_name_strategy(), user in "[a-z0-9_]{1,30}") {
        let b = flag_kit::bucket(&flag, &user);
        prop_assert!(b < 100, "bucket must be <100 got {b}");
    }

    #[test]
    fn bucket_deterministic(flag in valid_flag_name_strategy(), user in "[a-z0-9_]{1,30}") {
        let b1 = flag_kit::bucket(&flag, &user);
        let b2 = flag_kit::bucket(&flag, &user);
        prop_assert_eq!(b1, b2);
    }

    #[test]
    fn flag_serde_roundtrip(name in valid_flag_name_strategy(), pct in 0u8..=100, enabled in prop::bool::ANY) {
        let flag = Flag::new(FlagName::new(name).unwrap(), enabled, pct).unwrap();
        let json = serde_json::to_string(&flag).unwrap();
        let parsed: Flag = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(flag, parsed);
    }

    #[test]
    fn flag_change_serde_roundtrip(name in valid_flag_name_strategy(), old in prop::bool::ANY, new in prop::bool::ANY, when in -1_000_000i64..2_000_000_000i64) {
        let c = flag_kit::FlagChange::new(FlagName::new(name).unwrap(), old, new, "tester", when);
        let json = serde_json::to_string(&c).unwrap();
        let parsed: flag_kit::FlagChange = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(c, parsed);
    }
}

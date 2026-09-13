//! Equivalence proof: `flag_kit::FlagName` and `validkit::FlagName` must
//! accept and reject exactly the same set of names.
//!
//! flag-kit 0.2.0 deleted its local ~40-line copy of the
//! `^[a-z][a-z0-9_]*$` validation and delegates to `validkit::FlagName`.
//! The oracle is the historical flag-kit behavior pinned by the existing
//! suites (`src/flag.rs` unit tests, `tests/proptest.rs`,
//! `tests/integration.rs`); these tests prove the delegated validator
//! reproduces it exactly — on the curated oracle sets, on an exhaustive
//! 1-char ASCII sweep, on an exhaustive 2-char sweep over a mixed
//! alphabet, and on structured adversarial cases — under every feature
//! combination (`--no-default-features` through `--all-features`, which
//! flips validkit's regex engine on and off inside `validkit/regex`).
//!
//! Tests assert on decisions; unwrap/expect are the test signal here.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use flag_kit::FlagName;

/// The decision functions under comparison.
fn flag_kit_accepts(s: &str) -> bool {
    FlagName::new(s.to_string()).is_ok()
}

fn validkit_accepts(s: &str) -> bool {
    validkit::FlagName::parse(s).is_ok()
}

fn assert_equivalent(s: &str) {
    assert_eq!(
        flag_kit_accepts(s),
        validkit_accepts(s),
        "accept/reject disagreement on {s:?}: flag-kit={}, validkit={}",
        flag_kit_accepts(s),
        validkit_accepts(s)
    );
}

// ---------------------------------------------------------------------------
// 1. The historical flag-kit oracle sets (from src/flag.rs unit tests)
// ---------------------------------------------------------------------------

#[test]
fn oracle_valid_set_still_accepted_and_matches_validkit() {
    for valid in ["a", "abc", "my_flag", "flag_123", "a1_b2_c3"] {
        assert!(flag_kit_accepts(valid), "historically valid: {valid}");
        assert_equivalent(valid);
    }
}

#[test]
fn oracle_invalid_set_still_rejected_and_matches_validkit() {
    for invalid in ["", "A", "1abc", "_abc", "abc-def", "abc def", "ABC"] {
        assert!(
            !flag_kit_accepts(invalid),
            "historically invalid: {invalid}"
        );
        assert_equivalent(invalid);
    }
}

// ---------------------------------------------------------------------------
// 2. Exhaustive single-character ASCII sweep
// ---------------------------------------------------------------------------

#[test]
fn exhaustive_single_char_ascii_agreement() {
    for b in 0u8..=127 {
        let s = b as char;
        assert_equivalent(&s.to_string());
    }
    // The accepted 1-char set must be exactly a-z (the historical rule).
    for c in 'a'..='z' {
        assert!(flag_kit_accepts(&c.to_string()), "1-char a-z valid: {c}");
    }
    for b in 0u8..=127 {
        let c = b as char;
        if !c.is_ascii_lowercase() {
            assert!(
                !flag_kit_accepts(&c.to_string()),
                "1-char non-a-z must be rejected: {c:?}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Exhaustive two-character sweep over a mixed alphabet
// ---------------------------------------------------------------------------

#[test]
fn exhaustive_two_char_mixed_alphabet_agreement() {
    // Alphabet mixing every character class the rule distinguishes:
    // lowercase, digit, underscore, plus historical reject triggers.
    let alphabet: Vec<char> = vec!['a', 'z', 'f', '0', '9', '_', '-', 'A', 'Z', ' ', '\n'];
    for a in &alphabet {
        for b in &alphabet {
            let s: String = [*a, *b].iter().collect();
            assert_equivalent(&s);
        }
    }
}

// ---------------------------------------------------------------------------
// 4. Structured adversarial cases (incl. validkit's CR/LF guard)
// ---------------------------------------------------------------------------

#[test]
fn structured_cases_agreement() {
    let cases = [
        "flag",
        "flag_name_2",
        "a",
        "a_",
        "_",
        "_a",
        "0a",
        "a-b",
        "a b",
        "Flag",
        "fLAG",
        "flag\r",
        "flag\n",
        "fla\rg",
        "\r",
        "\n",
        "\r\n",
        "café",
        "flag name",
        "flag\ttab",
        &"a".repeat(256),
        &format!("a{}", "0".repeat(1000)),
        "flag\x007f",
        "\u{00e9}abc",
        "abc\u{200b}",
    ];
    for s in cases {
        assert_equivalent(s);
    }
}

// ---------------------------------------------------------------------------
// 5. Accepted inputs agree on the wrapped value, not just the decision
// ---------------------------------------------------------------------------

#[test]
fn accepted_values_roundtrip_identically() {
    for s in ["a", "my_flag", "flag_123", "a1_b2_c3"] {
        let mine = FlagName::new(s.to_string()).unwrap();
        let theirs = validkit::FlagName::parse(s).unwrap();
        assert_eq!(mine.as_str(), theirs.as_str());
        assert_eq!(mine.to_string(), theirs.to_string());
        assert_eq!(mine.into_inner(), theirs.into_inner());
    }
}

// ---------------------------------------------------------------------------
// 6. All entry points delegate (FromStr / TryFrom / parse parity)
// ---------------------------------------------------------------------------

#[test]
fn every_entry_point_uses_the_same_validator() {
    for s in ["good_name", "Bad-Name", ""] {
        let expected = flag_kit_accepts(s);
        assert_eq!(s.parse::<FlagName>().is_ok(), expected, "FromStr: {s:?}");
        assert_eq!(
            FlagName::try_from(s.to_string()).is_ok(),
            expected,
            "TryFrom<String>: {s:?}"
        );
        assert_eq!(
            FlagName::try_from(s).is_ok(),
            expected,
            "TryFrom<&str>: {s:?}"
        );
        assert_eq!(validkit::is_valid_flag_name(s), validkit_accepts(s));
    }
}

// ---------------------------------------------------------------------------
// 7. Ordering agrees with the inner string order (validkit 1.3.1 Ord)
// ---------------------------------------------------------------------------

#[test]
fn ordering_matches_validkit() {
    let pairs = [
        ("a", "b"),
        ("flag_a", "flag_b"),
        ("z9", "z_"),
        ("ab", "abc"),
    ];
    for (x, y) in pairs {
        let mx = FlagName::new(x).unwrap();
        let my = FlagName::new(y).unwrap();
        let vx = validkit::FlagName::parse(x).unwrap();
        let vy = validkit::FlagName::parse(y).unwrap();
        assert_eq!(mx.cmp(&my), vx.cmp(&vy), "ordering mismatch {x:?} vs {y:?}");
    }
}

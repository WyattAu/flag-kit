# Changelog

All notable changes to this project are documented here. Format: [Keep a
Changelog](https://keepachangelog.com/) — versions follow [semver](https://semver.org).

## [Unreleased]

## [0.2.0] - 2026-09-13

### Changed

- **Flag-name validation is now delegated to `validkit::FlagName`**
  (dogfooding): the local ~40-line duplicate of the
  `^[a-z][a-z0-9_]*$` rule — hand-rolled char loop plus a second
  regex-engine copy behind the `regex` feature — is deleted. The
  accept/reject set is unchanged and now pinned equal to validkit's by
  `tests/equivalence.rs` (historical oracle sets, exhaustive 1-char
  ASCII sweep, exhaustive 2-char mixed-alphabet sweep, structured
  adversarial cases, ordering parity) under all feature combinations.

### Added

- `validkit` dependency (>= 1.3.1) as the single source of truth for
  flag-name validation.

### Removed

- The direct `regex` dependency. The `regex` feature remains, now
  forwarding to `validkit/regex` (regex-backed validation inside
  validkit); enabling it no longer links a second regex engine into
  flag-kit itself.

## [0.1.2] - 2026-09-12

### Added

- config-knob behavior matrix: tests/config_matrix.rs pins every public knob to observable behavior — enabled (global eval flip), percentage 0/partial/100 (rollout outcome + bucket predicate across 500 users), set_percentage (flips evaluator predicate both ways, rejects out-of-range without corrupting state), with_created_at (default None vs set, both chrono and non-chrono builds), and pins the documented org_id-is-inert contract.


## [0.1.0] - 2026-09-03

### Added
- Feature flags with deterministic percentage rollout, targeting, and audit.
- Published to crates.io (2026-09-03).

## [0.1.1] - 2026-09-05

### Fixed
- Aligned rusqlite to 0.32 for ferro workspace compatibility (links=sqlite3 conflict)

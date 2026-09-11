# Requirements — flag-kit

Numbered, testable requirements. Every requirement maps to at least one named
test or doc-comment contract; security-relevant items cite threat-model rows.

Scope: Feature flags (`flag-kit`) — typed boolean/percentage/user-targeted flags with reload

## Functional

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-FK-001 | Flag evaluation is pure given (flags, user key): same inputs → same result | MUST |
| REQ-FK-002 | Percentage rollout uses stable hashing of the user key (no modulo bias between buckets) | MUST |
| REQ-FK-003 | Backends (sqlite) are feature-gated; reload is atomic (no torn state) | MUST |

## Security

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-FK-100 | Hash-based bucketing cannot be inverted to recover user keys | SHOULD |

## Observability & API hygiene

| ID | Requirement | Priority |
|----|-------------|----------|
| REQ-FK-900 | All fallible public APIs return typed errors; production `unwrap`/`expect` is denied or explicitly justified with an invariant comment | MUST |
| REQ-FK-901 | Public items carry doc comments with runnable examples where practical | SHOULD |

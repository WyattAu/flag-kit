# Pattern adoption

Relationship to the WyattAu pattern canon
(`github.com/WyattAu/pattern-standards`), validated by
`pattern-standards/scripts/adoption-check.py`.

| Pattern | Status | Evidence |
|---|---|---|
| T1 | adopted | src/flag.rs:21 (FlagName validates once at construction; the rule is delegated to validkit at src/flag.rs:49) |
| T13 | partial | src/eval.rs:142 (pure bucket function), src/store.rs:15 (IO behind the FlagStore trait) (the Evaluator still interleaves store reads into evaluation) |
| P44 | planned | store-layer pruning keyed on created_at (src/flag.rs:112); the library owns no scheduler, target 2027-Q1 |
| P47 | planned | snapshot-consistent reads for multi-flag evaluation, target 2027-Q1 |
| P57 | adopted | tests/equivalence.rs:1 (accept/reject parity with validkit on exhaustive and adversarial sweeps, across feature combinations) |
| P64 | adopted | tests/proptest.rs:1 (name acceptance, roundtrip, and percentage range properties) |
| P78 | partial | src/eval.rs:98 (deterministic sticky percentage rollout) (no named cohorts or staged groups) |
| P83 | partial | src/store.rs:166 (append-only flag_changes audit table with who/when) (no hash chain; records are not tamper-evident) |
| P96 | partial | src/store.rs:246 and src/store.rs:318 (re-validation when reading flags back from SQLite) (the write-side re-check at src/store.rs:268 duplicates the constructor guarantee) |

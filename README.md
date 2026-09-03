# flag-kit

Feature flags with rollout, targeting, and audit — deterministic percentage rollout, org targeting, and change history.

## Features

- **Validation**: Flag names `^[a-z][a-z0-9_]*$`
- **Rollout**: Deterministic SipHash bucket `hash(flag_name + user_id) % 100 < percentage`
- **Targeting**: `org_id` aware `enabled_for`
- **Audit**: `FlagChange` records with SQLite persistence
- **Stores**: `MemoryFlagStore` (DashMap) and `SqliteFlagStore` (rusqlite bundled)

## Usage

```rust
use flag_kit::{Flag, FlagName, MemoryFlagStore, Evaluator};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let store = Arc::new(MemoryFlagStore::new());
    store.set(Flag::new(FlagName::new("new_checkout")?, true, 50)?).await?;

    let eval = Evaluator::new(store);
    let enabled = eval.enabled_for(&FlagName::new("new_checkout")?, "user_123", None).await;
    println!("enabled: {enabled}");
    Ok(())
}
```

## Feature Flags

- `chrono` — `DateTime<Utc>` for `Flag.created_at`
- `sqlite` — `SqliteFlagStore` with `rusqlite`
- `regex` — regex validation for `FlagName`
- `tracing` — instrumented store/eval

## License

MIT OR Apache-2.0

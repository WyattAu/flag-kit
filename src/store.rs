//! Storage backends for flags.

use crate::error::{FlagError, Result};
use crate::flag::{Flag, FlagName};
use dashmap::DashMap;
use std::sync::Arc;

#[cfg(feature = "sqlite")]
use std::path::Path;

/// Trait for flag storage.
///
/// Implementations must be `Send + Sync` to allow sharing across tasks.
#[async_trait::async_trait]
pub trait FlagStore: Send + Sync {
    /// Get a flag by name, returning `None` if not found.
    async fn get(&self, name: &FlagName) -> Option<Flag>;

    /// Insert or update a flag.
    async fn set(&self, flag: Flag) -> Result<()>;

    /// List all flags.
    async fn list(&self) -> Vec<Flag>;

    /// Delete a flag by name, returning `true` if existed.
    async fn delete(&self, name: &FlagName) -> Result<bool> {
        let _ = name;
        Err(FlagError::Storage("delete not implemented".to_string()))
    }
}

// ---------------------------------------------------------------------------
// In-memory store
// ---------------------------------------------------------------------------

/// In-memory flag store backed by `DashMap`.
///
/// Suitable for tests, single-process deployments, and as a cache layer.
#[derive(Debug, Default)]
pub struct MemoryFlagStore {
    inner: DashMap<String, Flag>,
}

impl MemoryFlagStore {
    /// Creates a new empty store.
    pub fn new() -> Self {
        Self {
            inner: DashMap::new(),
        }
    }

    /// Number of flags in store.
    pub fn len(&self) -> usize {
        self.inner.len()
    }

    /// Returns `true` if store is empty.
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Clear all flags.
    pub fn clear(&self) {
        self.inner.clear();
    }
}

#[async_trait::async_trait]
impl FlagStore for MemoryFlagStore {
    async fn get(&self, name: &FlagName) -> Option<Flag> {
        #[cfg(feature = "tracing")]
        tracing::trace!(flag = %name, "memory get");
        self.inner.get(name.as_str()).map(|r| r.clone())
    }

    async fn set(&self, flag: Flag) -> Result<()> {
        #[cfg(feature = "tracing")]
        tracing::debug!(flag = %flag.name, enabled = flag.enabled, percentage = flag.percentage, "memory set");
        self.inner.insert(flag.name.as_str().to_string(), flag);
        Ok(())
    }

    async fn list(&self) -> Vec<Flag> {
        self.inner.iter().map(|kv| kv.value().clone()).collect()
    }

    async fn delete(&self, name: &FlagName) -> Result<bool> {
        Ok(self.inner.remove(name.as_str()).is_some())
    }
}

// Ensure MemoryFlagStore can be used via Arc<dyn FlagStore>.
#[async_trait::async_trait]
impl FlagStore for Arc<MemoryFlagStore> {
    async fn get(&self, name: &FlagName) -> Option<Flag> {
        (**self).get(name).await
    }
    async fn set(&self, flag: Flag) -> Result<()> {
        (**self).set(flag).await
    }
    async fn list(&self) -> Vec<Flag> {
        (**self).list().await
    }
    async fn delete(&self, name: &FlagName) -> Result<bool> {
        (**self).delete(name).await
    }
}

// ---------------------------------------------------------------------------
// SQLite store
// ---------------------------------------------------------------------------

/// SQLite-backed flag store.
///
/// Requires the `sqlite` feature.
#[cfg(feature = "sqlite")]
pub struct SqliteFlagStore {
    conn: Arc<tokio::sync::Mutex<rusqlite::Connection>>,
}

#[cfg(feature = "sqlite")]
impl SqliteFlagStore {
    /// Opens or creates a SQLite database at `path`.
    ///
    /// Creates the `flags` table if it does not exist.
    pub fn new(path: impl AsRef<Path>) -> Result<Self> {
        let conn = rusqlite::Connection::open(path.as_ref())
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(tokio::sync::Mutex::new(conn)),
        })
    }

    /// Creates an in-memory SQLite database (useful for tests).
    pub fn in_memory() -> Result<Self> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(tokio::sync::Mutex::new(conn)),
        })
    }

    /// Creates from an existing `rusqlite::Connection`.
    pub fn from_connection(conn: rusqlite::Connection) -> Result<Self> {
        Self::init(&conn)?;
        Ok(Self {
            conn: Arc::new(tokio::sync::Mutex::new(conn)),
        })
    }

    fn init(conn: &rusqlite::Connection) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS flags (
                name TEXT PRIMARY KEY,
                enabled INTEGER NOT NULL,
                percentage INTEGER NOT NULL,
                created_at INTEGER
            )",
            [],
        )
        .map_err(|e| FlagError::Storage(e.to_string()))?;
        // For audit, optional table
        conn.execute(
            "CREATE TABLE IF NOT EXISTS flag_changes (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                name TEXT NOT NULL,
                old INTEGER NOT NULL,
                new INTEGER NOT NULL,
                who TEXT NOT NULL,
                \"when\" INTEGER NOT NULL
            )",
            [],
        )
        .map_err(|e| FlagError::Storage(e.to_string()))?;
        Ok(())
    }

    /// Record a change for audit.
    pub async fn record_change(&self, change: &crate::flag::FlagChange) -> Result<()> {
        let c = change.clone();
        let conn = self.conn.clone();
        // rusqlite is synchronous; hold mutex briefly
        let guard = conn.lock().await;
        guard
            .execute(
                "INSERT INTO flag_changes (name, old, new, who, \"when\") VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![c.name.as_str(), c.old as i64, c.new as i64, c.who, c.when],
            )
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        Ok(())
    }

    /// List audit changes for a flag, newest first.
    pub async fn list_changes(&self, name: &FlagName) -> Result<Vec<crate::flag::FlagChange>> {
        let name_str = name.as_str().to_string();
        let conn = self.conn.clone();
        let guard = conn.lock().await;
        let mut stmt = guard
            .prepare("SELECT name, old, new, who, \"when\" FROM flag_changes WHERE name = ?1 ORDER BY \"when\" DESC")
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        let rows = stmt
            .query_map(rusqlite::params![name_str], |row| {
                let name_s: String = row.get(0)?;
                let old: i64 = row.get(1)?;
                let new: i64 = row.get(2)?;
                let who: String = row.get(3)?;
                let when: i64 = row.get(4)?;
                Ok(crate::flag::FlagChange {
                    name: FlagName::new_unchecked(name_s),
                    old: old != 0,
                    new: new != 0,
                    who,
                    when,
                })
            })
            .map_err(|e| FlagError::Storage(e.to_string()))?;

        let mut out = Vec::new();
        for r in rows {
            out.push(r.map_err(|e| FlagError::Storage(e.to_string()))?);
        }
        Ok(out)
    }
}

#[cfg(feature = "sqlite")]
#[async_trait::async_trait]
impl FlagStore for SqliteFlagStore {
    async fn get(&self, name: &FlagName) -> Option<Flag> {
        #[cfg(feature = "tracing")]
        tracing::trace!(flag = %name, "sqlite get");
        let name_str = name.as_str().to_string();
        let conn = self.conn.clone();
        let guard = conn.lock().await;
        let mut stmt = guard
            .prepare("SELECT name, enabled, percentage, created_at FROM flags WHERE name = ?1")
            .ok()?;
        let mut rows = stmt.query(rusqlite::params![name_str]).ok()?;
        let row = rows.next().ok()??;
        let name_s: String = row.get(0).ok()?;
        let enabled: i64 = row.get(1).ok()?;
        let percentage: i64 = row.get(2).ok()?;
        let created_at: Option<i64> = row.get(3).ok()?;
        let flag_name = FlagName::new(name_s).ok()?;
        let pct = u8::try_from(percentage).ok()?;
        if pct > 100 {
            return None;
        }
        Some(Flag {
            name: flag_name,
            enabled: enabled != 0,
            percentage: pct,
            #[cfg(feature = "chrono")]
            created_at: created_at.map(|ts| {
                use chrono::{TimeZone, Utc};
                Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
            }),
            #[cfg(not(feature = "chrono"))]
            created_at,
        })
    }

    async fn set(&self, flag: Flag) -> Result<()> {
        #[cfg(feature = "tracing")]
        tracing::debug!(flag = %flag.name, enabled = flag.enabled, percentage = flag.percentage, "sqlite set");
        if flag.percentage > 100 {
            return Err(FlagError::Storage(format!(
                "percentage must be 0..=100, got {}",
                flag.percentage
            )));
        }
        let conn = self.conn.clone();
        let guard = conn.lock().await;
        let ts: Option<i64> = {
            #[cfg(feature = "chrono")]
            {
                flag.created_at.map(|dt| dt.timestamp())
            }
            #[cfg(not(feature = "chrono"))]
            {
                flag.created_at
            }
        };
        guard
            .execute(
                "INSERT INTO flags (name, enabled, percentage, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(name) DO UPDATE SET enabled=excluded.enabled, percentage=excluded.percentage, created_at=excluded.created_at",
                rusqlite::params![flag.name.as_str(), flag.enabled as i64, flag.percentage as i64, ts],
            )
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        Ok(())
    }

    async fn list(&self) -> Vec<Flag> {
        let conn = self.conn.clone();
        let guard = conn.lock().await;
        let mut stmt =
            match guard.prepare("SELECT name, enabled, percentage, created_at FROM flags") {
                Ok(s) => s,
                Err(_) => return Vec::new(),
            };
        let rows = match stmt.query_map([], |row| {
            let name_s: String = row.get(0)?;
            let enabled: i64 = row.get(1)?;
            let percentage: i64 = row.get(2)?;
            let created_at: Option<i64> = row.get(3)?;
            Ok((name_s, enabled, percentage, created_at))
        }) {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };

        let mut out = Vec::new();
        for r in rows.flatten() {
            let (name_s, enabled, percentage, created_at) = r;
            if let Ok(fname) = FlagName::new(name_s) {
                if let Ok(pct) = u8::try_from(percentage) {
                    if pct <= 100 {
                        out.push(Flag {
                            name: fname,
                            enabled: enabled != 0,
                            percentage: pct,
                            #[cfg(feature = "chrono")]
                            created_at: created_at.map(|ts| {
                                use chrono::{TimeZone, Utc};
                                Utc.timestamp_opt(ts, 0).single().unwrap_or_else(Utc::now)
                            }),
                            #[cfg(not(feature = "chrono"))]
                            created_at,
                        });
                    }
                }
            }
        }
        out
    }

    async fn delete(&self, name: &FlagName) -> Result<bool> {
        let conn = self.conn.clone();
        let guard = conn.lock().await;
        let n = guard
            .execute(
                "DELETE FROM flags WHERE name = ?1",
                rusqlite::params![name.as_str()],
            )
            .map_err(|e| FlagError::Storage(e.to_string()))?;
        Ok(n > 0)
    }
}

#[cfg(feature = "sqlite")]
impl core::fmt::Debug for SqliteFlagStore {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SqliteFlagStore").finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flag::FlagName;

    #[tokio::test]
    async fn memory_store_basic() {
        let store = MemoryFlagStore::new();
        let name = FlagName::new("test_flag").unwrap();
        assert!(store.get(&name).await.is_none());
        let flag = Flag::new(name.clone(), true, 50).unwrap();
        store.set(flag.clone()).await.unwrap();
        let got = store.get(&name).await.unwrap();
        assert_eq!(got.name, flag.name);
        assert_eq!(got.percentage, 50);
        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert!(store.delete(&name).await.unwrap());
        assert!(store.get(&name).await.is_none());
    }

    #[tokio::test]
    async fn memory_store_overwrite() {
        let store = MemoryFlagStore::new();
        let name = FlagName::new("overwrite").unwrap();
        store
            .set(Flag::new(name.clone(), true, 10).unwrap())
            .await
            .unwrap();
        store
            .set(Flag::new(name.clone(), false, 90).unwrap())
            .await
            .unwrap();
        let got = store.get(&name).await.unwrap();
        assert!(!got.enabled);
        assert_eq!(got.percentage, 90);
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn sqlite_in_memory_basic() {
        let store = SqliteFlagStore::in_memory().unwrap();
        let name = FlagName::new("sqlite_flag").unwrap();
        let flag = Flag::new(name.clone(), true, 25).unwrap();
        store.set(flag).await.unwrap();
        let got = store.get(&name).await.unwrap();
        assert_eq!(got.percentage, 25);
        assert!(got.enabled);
        let list = store.list().await;
        assert_eq!(list.len(), 1);
        assert!(store.delete(&name).await.unwrap());
        assert!(store.get(&name).await.is_none());
    }

    #[cfg(feature = "sqlite")]
    #[tokio::test]
    async fn sqlite_persistence_file() {
        let dir = std::env::temp_dir().join("flag_kit_test_sqlite");
        let db = dir.join("test.db");
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::create_dir_all(&dir);
        {
            let s = SqliteFlagStore::new(&db).unwrap();
            s.set(Flag::new(FlagName::new("persist").unwrap(), true, 100).unwrap())
                .await
                .unwrap();
        }
        {
            let s = SqliteFlagStore::new(&db).unwrap();
            let f = s.get(&FlagName::new("persist").unwrap()).await.unwrap();
            assert_eq!(f.percentage, 100);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

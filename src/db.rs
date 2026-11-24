use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use thiserror::Error;

use crate::model;

const MIGRATIONS: &[(&str, &str)] = &[("0001", include_str!("../migrations/0001_init.sql"))];

/// An open SQLite database, with migrations applied and WAL mode on.
///
/// All access goes through a single connection guarded by a mutex; SQLite is
/// fast enough for this single-writer service that a pool is unnecessary.
pub struct Db {
    conn: Mutex<Connection>,
}

#[derive(Debug, Error)]
pub enum DbError {
    #[error("failed to open database at {0}: {1}")]
    Open(String, #[source] rusqlite::Error),
    #[error("failed to run database migrations: {0}")]
    Migration(#[source] rusqlite::Error),
    #[error("failed to read the inventory: {0}")]
    Query(#[source] rusqlite::Error),
}

impl Db {
    /// Open (creating when absent) the database at `path`, apply every
    /// embedded migration, and switch the database to WAL mode.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, DbError> {
        let path = path.as_ref();
        let mut conn =
            Connection::open(path).map_err(|error| DbError::Open(path_display(path), error))?;
        Self::migrate(&mut conn)?;
        conn.execute_batch("PRAGMA journal_mode = WAL")
            .map_err(DbError::Migration)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run every embedded migration in order, inside one transaction.
    fn migrate(conn: &mut Connection) -> Result<(), DbError> {
        let tx = conn.transaction().map_err(DbError::Migration)?;
        for (_name, sql) in MIGRATIONS {
            tx.execute_batch(sql).map_err(DbError::Migration)?;
        }
        tx.commit().map_err(DbError::Migration)
    }

    /// Return every known Device, most recently seen first.
    pub fn list_devices(&self) -> Result<Vec<model::Device>, DbError> {
        let conn = self
            .conn
            .lock()
            .expect("database connection mutex is not poisoned");
        let mut stmt = conn
            .prepare(
                "SELECT mac, ip, hostname, first_seen, last_seen FROM devices ORDER BY last_seen DESC",
            )
            .map_err(DbError::Query)?;
        let rows = stmt
            .query_map([], |row| {
                Ok(model::Device {
                    mac: row.get(0)?,
                    ip: row.get(1)?,
                    hostname: row.get(2)?,
                    first_seen: row.get::<_, i64>(3)? as u64,
                    last_seen: row.get::<_, i64>(4)? as u64,
                })
            })
            .map_err(DbError::Query)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Query)
    }
}

fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

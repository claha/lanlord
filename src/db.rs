use std::path::Path;
use std::sync::Mutex;

use rusqlite::Connection;
use rusqlite::OptionalExtension;
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
    #[error("failed to write to the inventory: {0}")]
    Write(#[source] rusqlite::Error),
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
                    mac: row.get::<_, String>(0)?.into(),
                    ip: row.get(1)?,
                    hostname: row.get(2)?,
                    first_seen: row.get::<_, i64>(3)? as u64,
                    last_seen: row.get::<_, i64>(4)? as u64,
                    present: false, // derived at render time from the last scan start
                })
            })
            .map_err(DbError::Query)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(DbError::Query)
    }

    /// Upsert a Device observed in a Scan: insert a new Device with First seen
    /// and Last seen set to `now`; on an existing MAC, refresh the IP, the
    /// hostname and Last seen but never touch First seen. A `None` hostname
    /// (failed reverse lookup) stores nothing and leaves the other fields as
    /// they were.
    pub fn upsert_device(
        &self,
        mac: &model::Mac,
        ip: &str,
        hostname: Option<&str>,
        now: u64,
    ) -> Result<(), DbError> {
        let conn = self
            .conn
            .lock()
            .expect("database connection mutex is not poisoned");
        // A failed reverse lookup (NULL hostname) must not wipe a previously
        // resolved name; only store a hostname when the new observation
        // actually resolved one.
        conn.execute(
            "INSERT INTO devices (mac, ip, hostname, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?4)
             ON CONFLICT(mac) DO UPDATE SET
                 ip = excluded.ip,
                 hostname = CASE WHEN excluded.hostname IS NOT NULL
                             THEN excluded.hostname
                             ELSE devices.hostname END,
                 last_seen = excluded.last_seen",
            (mac.as_ref(), ip, hostname, now),
        )
        .map_err(DbError::Write)?;
        Ok(())
    }

    /// Record the start time of the most recent Scan in the `meta` table.
    pub fn set_last_scan_start(&self, now: u64) -> Result<(), DbError> {
        let conn = self
            .conn
            .lock()
            .expect("database connection mutex is not poisoned");
        conn.execute(
            "INSERT INTO meta (key, value) VALUES ('last_scan_start', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [now.to_string()],
        )
        .map_err(DbError::Write)?;
        Ok(())
    }

    /// Return the start time of the most recent Scan, if any scan has run.
    pub fn last_scan_start(&self) -> Result<Option<u64>, DbError> {
        let conn = self
            .conn
            .lock()
            .expect("database connection mutex is not poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM meta WHERE key = 'last_scan_start'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(DbError::Query)?;
        Ok(value.map(|text| {
            text.parse::<u64>()
                .expect("meta value is a valid epoch seconds")
        }))
    }
}

fn path_display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_test_db(name: &str) -> Db {
        let path = std::env::temp_dir().join(name);
        let _ = std::fs::remove_file(&path); // ignore missing
        Db::open(&path).expect("test database opens")
    }

    #[test]
    fn upsert_device_inserts_new_device_with_first_and_last_seen() {
        let db = open_test_db("lanlord.test.db.upsert-insert.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(&model::Mac::new("aa:bb:cc:dd:ee:ff"), "10.0.0.1", None, now)
            .expect("insert succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].mac, model::Mac::new("aa:bb:cc:dd:ee:ff"));
        assert_eq!(devices[0].ip, "10.0.0.1");
        assert_eq!(devices[0].hostname, None);
        assert_eq!(devices[0].first_seen, now);
        assert_eq!(devices[0].last_seen, now);
    }

    #[test]
    fn upsert_device_stores_hostname_on_insert() {
        let db = open_test_db("lanlord.test.db.hostname-insert.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.1",
            Some("gateway.home.arpa"),
            now,
        )
        .expect("insert succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].mac, model::Mac::new("aa:bb:cc:dd:ee:ff"));
        assert_eq!(devices[0].ip, "10.0.0.1");
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("gateway.home.arpa"),
            "resolved hostname is stored"
        );
    }

    #[test]
    fn upsert_device_repeat_updates_ip_and_last_seen_keeps_first_seen() {
        let db = open_test_db("lanlord.test.db.upsert-repeat.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(&model::Mac::new("aa:bb:cc:dd:ee:ff"), "10.0.0.1", None, now)
            .expect("first insert succeeds");
        let later = now + 10;
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.2",
            None,
            later,
        )
        .expect("repeat upsert succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1, "no duplicate device is created");
        assert_eq!(devices[0].mac, model::Mac::new("aa:bb:cc:dd:ee:ff"));
        assert_eq!(devices[0].ip, "10.0.0.2", "most recent IP wins");
        assert_eq!(
            devices[0].first_seen, now,
            "first seen is never overwritten"
        );
        assert_eq!(devices[0].last_seen, later, "last seen is refreshed");
    }

    #[test]
    fn upsert_device_refreshes_hostname_when_it_changes() {
        let db = open_test_db("lanlord.test.db.hostname-refresh.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.1",
            Some("old.home.arpa"),
            now,
        )
        .expect("first upsert succeeds");
        let first_seen = db.list_devices().expect("list succeeds")[0].first_seen;
        let later = now + 10;
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.2",
            Some("new.home.arpa"),
            later,
        )
        .expect("repeat upsert succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].ip, "10.0.0.2", "most recent IP wins");
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("new.home.arpa"),
            "hostname is refreshed"
        );
        assert_eq!(
            devices[0].first_seen, first_seen,
            "first seen is never overwritten"
        );
    }

    #[test]
    fn upsert_device_without_hostname_keeps_existing_hostname() {
        let db = open_test_db("lanlord.test.db.hostname-none.sqlite");
        let now = model::now_epoch_seconds();
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.1",
            Some("gateway.home.arpa"),
            now,
        )
        .expect("first upsert succeeds");
        let later = now + 10;
        db.upsert_device(
            &model::Mac::new("aa:bb:cc:dd:ee:ff"),
            "10.0.0.2",
            None,
            later,
        )
        .expect("repeat upsert without hostname succeeds");
        let devices = db.list_devices().expect("list succeeds");
        assert_eq!(devices.len(), 1);
        assert_eq!(
            devices[0].hostname.as_deref(),
            Some("gateway.home.arpa"),
            "existing hostname is preserved when the lookup fails"
        );
        assert_eq!(devices[0].ip, "10.0.0.2", "ip is still refreshed");
        assert_eq!(devices[0].last_seen, later, "last seen is still refreshed");
    }

    #[test]
    fn last_scan_start_is_none_before_any_scan() {
        let db = open_test_db("lanlord.test.db.meta-none.sqlite");
        assert_eq!(
            db.last_scan_start().expect("read succeeds"),
            None,
            "no scan start time is recorded before the first scan"
        );
    }

    #[test]
    fn last_scan_start_is_written_and_read_back() {
        let db = open_test_db("lanlord.test.db.meta-write.sqlite");
        db.set_last_scan_start(1_700_000_000)
            .expect("write succeeds");
        assert_eq!(
            db.last_scan_start().expect("read succeeds"),
            Some(1_700_000_000),
            "read back matches what was written"
        );

        db.set_last_scan_start(1_700_000_100)
            .expect("second write succeeds");
        assert_eq!(
            db.last_scan_start().expect("read succeeds"),
            Some(1_700_000_100),
            "second write overwrites the first"
        );
    }
}

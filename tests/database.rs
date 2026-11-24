use lanlord::db::Db;
use rusqlite::Connection;

#[test]
fn open_creates_database_file_applies_migrations_and_enables_wal() {
    let path = std::env::temp_dir().join(format!("lanlord.test.db-{}.sqlite", std::process::id()));
    let _ = std::fs::remove_file(&path);
    assert!(!path.exists(), "database file does not exist yet");

    let _db = Db::open(&path).expect("database opens");
    assert!(path.exists(), "database file is created when absent");

    // Migrations ran: the devices and meta tables exist.
    let conn = Connection::open(&path).expect("database can be re-opened for inspection");
    let tables: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("query prepares")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query executes")
        .map(|result| result.expect("row reads"))
        .collect();
    assert!(
        tables.contains(&"devices".into()),
        "devices table exists: {tables:?}"
    );
    assert!(
        tables.contains(&"meta".into()),
        "meta table exists: {tables:?}"
    );

    // WAL mode is on.
    let mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .expect("pragma reads");
    assert_eq!(mode, "wal", "database runs in WAL mode: {mode}");

    // Migrations run on every start: re-opening succeeds and the schema is stable.
    let _db = Db::open(&path).expect("database re-opens and migrations re-run");
    let tables_again: Vec<String> = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
        .expect("query prepares")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("query executes")
        .map(|result| result.expect("row reads"))
        .collect();
    assert_eq!(
        tables, tables_again,
        "schema is unchanged after re-running migrations"
    );

    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(format!("{}-wal", path.display()));
    let _ = std::fs::remove_file(format!("{}-shm", path.display()));
}

#[test]
fn open_reads_empty_inventory() {
    let path = std::env::temp_dir().join(format!(
        "lanlord.test.inventory-{}.sqlite",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let db = Db::open(&path).expect("database opens");
    let devices = db.list_devices().expect("devices can be listed");
    assert!(
        devices.is_empty(),
        "inventory is empty before any scan: {devices:?}"
    );
    let _ = std::fs::remove_file(&path);
}

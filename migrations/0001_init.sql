-- Schema for lanlord v1.
-- Applied in a transaction on every start; safe to re-run.

CREATE TABLE IF NOT EXISTS devices (
    mac     TEXT NOT NULL PRIMARY KEY,
    ip      TEXT NOT NULL,
    hostname TEXT,
    first_seen INTEGER NOT NULL, -- UTC epoch seconds
    last_seen  INTEGER NOT NULL  -- UTC epoch seconds
);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT NOT NULL PRIMARY KEY,
    value TEXT NOT NULL
);

//! The SQLite store. Only the daemon opens this.
//!
//! No migration framework: the schema is one table created idempotently on
//! every open. If it ever needs a column, that is an `ALTER TABLE` guarded by a
//! `PRAGMA table_info` check rather than a migrations directory.
//!
//! The daemon keeps the live timer list in memory and treats this as the
//! durable mirror — every mutation writes both — so a restart reloads exactly
//! what was pending, including anything whose deadline passed while it was down.

use anyhow::{Context, Result};
use rusqlite::{Connection, params};

use crate::models::Timer;

/// Open the database, creating the directory and the schema if needed.
pub fn open(path: &std::path::Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let db = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
    init(&db)?;
    Ok(db)
}

/// Create the schema if it is not already there, and bring an older one up to
/// date.
pub fn init(db: &Connection) -> Result<()> {
    db.execute_batch(
        // AUTOINCREMENT rather than a bare INTEGER PRIMARY KEY on purpose:
        // without it SQLite reuses the highest rowid once a row is deleted, so
        // a bar that just read id 4 for "tea" could ack a brand-new "pasta"
        // that inherited the id in between.
        "CREATE TABLE IF NOT EXISTS timers (
            id         INTEGER PRIMARY KEY AUTOINCREMENT,
            name       TEXT    NOT NULL,
            deadline   INTEGER NOT NULL,
            created_at INTEGER NOT NULL,
            fired_at   INTEGER,
            private    INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS timers_deadline ON timers (deadline);",
    )?;
    // The CREATE above only shapes a *new* database. A database from before
    // `private` existed is left alone by CREATE TABLE IF NOT EXISTS, so the
    // column has to be added to it explicitly.
    ensure_column(db, "private", "INTEGER NOT NULL DEFAULT 0")?;
    Ok(())
}

/// Add a column to `timers` unless it is already there.
///
/// SQLite has no `ADD COLUMN IF NOT EXISTS`, hence the `pragma_table_info`
/// check; running this on an up-to-date database has to be free, because it runs
/// on every open. `ADD COLUMN` with a `NOT NULL DEFAULT` is legal precisely
/// because the default tells SQLite what every existing row should hold, and it
/// rewrites nothing — so timers made before this feature existed come back
/// visible, which is the only safe reading of a column they never had.
fn ensure_column(db: &Connection, name: &str, decl: &str) -> Result<()> {
    let present: bool = db.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('timers') WHERE name = ?1)",
        params![name],
        |row| row.get(0),
    )?;
    if !present {
        db.execute_batch(&format!("ALTER TABLE timers ADD COLUMN {name} {decl}"))?;
    }
    Ok(())
}

/// Every timer, ordered the way clients want to show them: unacknowledged
/// alarms first (oldest alarm at the top), then whatever is counting down,
/// soonest first.
pub fn list(db: &Connection) -> Result<Vec<Timer>> {
    let mut stmt = db.prepare(
        "SELECT id, name, deadline, created_at, fired_at, private
           FROM timers
          ORDER BY fired_at IS NULL, fired_at, deadline, id",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(Timer {
            id: row.get(0)?,
            name: row.get(1)?,
            deadline: row.get(2)?,
            created_at: row.get(3)?,
            fired_at: row.get(4)?,
            // Read as an integer and compared, rather than as a bool: SQLite
            // does not police the column's type, and a hand-edited row holding
            // something other than 0 or 1 should not fail the whole query.
            private: row.get::<_, i64>(5)? != 0,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Insert a timer and return its id.
pub fn insert(
    db: &Connection,
    name: &str,
    deadline: i64,
    created_at: i64,
    private: bool,
) -> Result<i64> {
    db.execute(
        "INSERT INTO timers (name, deadline, created_at, fired_at, private)
         VALUES (?1, ?2, ?3, NULL, ?4)",
        params![name, deadline, created_at, private],
    )?;
    Ok(db.last_insert_rowid())
}

/// Delete one timer. Returns how many rows went, so the caller can tell a real
/// delete from a no-op without a second query.
pub fn delete(db: &Connection, id: i64) -> Result<usize> {
    Ok(db.execute("DELETE FROM timers WHERE id = ?1", params![id])?)
}

/// Delete every timer that has fired and not been acknowledged.
pub fn delete_fired(db: &Connection) -> Result<usize> {
    Ok(db.execute("DELETE FROM timers WHERE fired_at IS NOT NULL", [])?)
}

/// Record that a timer's deadline passed and its notification went out.
pub fn mark_fired(db: &Connection, id: i64, at: i64) -> Result<()> {
    db.execute(
        "UPDATE timers SET fired_at = ?2 WHERE id = ?1 AND fired_at IS NULL",
        params![id, at],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Connection {
        let db = Connection::open_in_memory().unwrap();
        init(&db).unwrap();
        db
    }

    #[test]
    fn init_is_idempotent() {
        let db = db();
        init(&db).unwrap();
        init(&db).unwrap();
        assert!(list(&db).unwrap().is_empty());
    }

    #[test]
    fn list_puts_alarms_before_countdowns() {
        let db = db();
        let tea = insert(&db, "tea", 500, 0, false).unwrap();
        let pasta = insert(&db, "pasta", 900, 0, false).unwrap();
        insert(&db, "bread", 100, 0, false).unwrap();
        mark_fired(&db, pasta, 950).unwrap();

        let names: Vec<String> = list(&db).unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(
            names,
            ["pasta", "bread", "tea"],
            "fired first, then by deadline"
        );
        assert_eq!(delete(&db, tea).unwrap(), 1);
        assert_eq!(delete(&db, tea).unwrap(), 0, "deleting twice is a no-op");
    }

    #[test]
    fn ids_are_never_reused() {
        let db = db();
        let first = insert(&db, "tea", 500, 0, false).unwrap();
        delete(&db, first).unwrap();
        let second = insert(&db, "pasta", 500, 0, false).unwrap();
        assert_ne!(
            first, second,
            "a recycled id would let a stale ack hit the wrong timer"
        );
    }

    #[test]
    fn private_survives_a_round_trip() {
        let db = db();
        insert(&db, "salary review", 500, 0, true).unwrap();
        insert(&db, "tea", 600, 0, false).unwrap();
        let timers = list(&db).unwrap();
        assert!(timers[0].private, "the hidden one came back visible");
        assert!(!timers[1].private);
    }

    /// The one test standing between this column and a database full of real
    /// timers: an older schema must gain the column without losing a row, and
    /// every existing timer must come back *visible*.
    #[test]
    fn an_older_database_gains_the_column_and_keeps_its_rows() {
        let db = Connection::open_in_memory().unwrap();
        // The four-column table exactly as it was before `private` existed.
        db.execute_batch(
            "CREATE TABLE timers (
                id         INTEGER PRIMARY KEY AUTOINCREMENT,
                name       TEXT    NOT NULL,
                deadline   INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                fired_at   INTEGER
            );",
        )
        .unwrap();
        db.execute_batch(
            "INSERT INTO timers (name, deadline, created_at, fired_at)
             VALUES ('from before', 500, 0, NULL);",
        )
        .unwrap();

        init(&db).unwrap();

        let timers = list(&db).unwrap();
        assert_eq!(timers.len(), 1, "the migration dropped a row");
        assert_eq!(timers[0].name, "from before");
        assert!(
            !timers[0].private,
            "a timer from before the column existed must default to visible"
        );

        // And running it again changes nothing — it happens on every open.
        init(&db).unwrap();
        assert_eq!(list(&db).unwrap().len(), 1);
    }

    #[test]
    fn mark_fired_does_not_move_an_existing_stamp() {
        let db = db();
        let id = insert(&db, "tea", 500, 0, false).unwrap();
        mark_fired(&db, id, 500).unwrap();
        mark_fired(&db, id, 900).unwrap();
        assert_eq!(list(&db).unwrap()[0].fired_at, Some(500));
        assert_eq!(delete_fired(&db).unwrap(), 1);
        assert!(list(&db).unwrap().is_empty());
    }
}

use rusqlite::Connection;

use crate::error::{DbError, Result};

/// Applies `migrations` (index + 1 is the version each one reaches) to `conn`, tracked in
/// `PRAGMA user_version`. Behind is adopted, equal is a no-op, ahead is refused.
pub(crate) fn migrate(
    conn: &mut Connection,
    label: &'static str,
    migrations: &[&str],
) -> Result<()> {
    let supported = migrations.len() as i64;
    let found: i64 = conn.pragma_query_value(None, "user_version", |r| r.get(0))?;
    if found > supported {
        return Err(DbError::NewerSchema {
            label,
            found,
            supported,
        });
    }
    for (i, sql) in migrations.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(c: &Connection) -> i64 {
        c.pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn migrations_apply_in_order_from_empty() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c, "t", &["CREATE TABLE a(x)", "CREATE TABLE b(y)"]).unwrap();
        assert_eq!(version(&c), 2);
        c.execute("INSERT INTO b VALUES (1)", []).unwrap();
    }

    #[test]
    fn a_behind_database_is_adopted_and_equal_is_a_no_op() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c, "t", &["CREATE TABLE a(x)"]).unwrap();
        migrate(&mut c, "t", &["CREATE TABLE a(x)"]).unwrap();
        migrate(&mut c, "t", &["CREATE TABLE a(x)", "CREATE TABLE b(y)"]).unwrap();
        assert_eq!(version(&c), 2);
    }

    #[test]
    fn a_newer_database_is_refused_untouched() {
        let mut c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "user_version", 5).unwrap();
        let e = migrate(&mut c, "t", &["CREATE TABLE a(x)"]).unwrap_err();
        assert!(matches!(
            e,
            DbError::NewerSchema {
                found: 5,
                supported: 1,
                ..
            }
        ));
        assert_eq!(version(&c), 5);
        assert!(c.prepare("SELECT * FROM a").is_err());
    }

    #[test]
    fn a_failing_migration_rolls_back_and_keeps_the_version() {
        let mut c = Connection::open_in_memory().unwrap();
        migrate(&mut c, "t", &["CREATE TABLE a(x)"]).unwrap();
        let bad = [
            "CREATE TABLE a(x)",
            "CREATE TABLE b(y); SELECT nope FROM missing",
        ];
        assert!(migrate(&mut c, "t", &bad).is_err());
        assert_eq!(version(&c), 1);
        assert!(c.prepare("SELECT * FROM b").is_err());
    }
}

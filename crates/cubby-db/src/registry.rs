use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension};

use crate::error::{DbError, Result};
use crate::{family, schema};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub id: String,
    pub email: String,
    pub created_at: i64,
}

pub const REGISTRY_VERSION: i64 = MIGRATIONS.len() as i64;

const MIGRATIONS: [&str; 2] = [
    "CREATE TABLE families(
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL UNIQUE,
    created_at INTEGER NOT NULL
)",
    "CREATE TABLE family_parents(
    family_id TEXT NOT NULL REFERENCES families(id),
    email TEXT NOT NULL UNIQUE,
    added_at INTEGER NOT NULL
);
INSERT INTO family_parents(family_id,email,added_at) SELECT id,email,created_at FROM families",
];

/// The file-per-tenant registry: `registry.sqlite` maps each parent email (a family has one or more)
/// to a family id, and the
/// family's data lives in `family-<id>.sqlite` beside it. A family's file is named by its minted
/// id, never by the email.
pub struct Registry {
    dir: PathBuf,
    conn: Connection,
}

fn valid_id(id: &str) -> bool {
    id.len() == 26
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_uppercase())
}

impl Registry {
    pub fn open(dir: &Path) -> Result<Registry> {
        std::fs::create_dir_all(dir)?;
        let mut conn = Connection::open(dir.join("registry.sqlite"))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        schema::migrate(&mut conn, "registry", &MIGRATIONS)?;
        Ok(Registry {
            dir: dir.to_path_buf(),
            conn,
        })
    }

    /// Registers `email` and provisions its family file. Row and file land together: a failure
    /// provisioning the file rolls the row back.
    pub fn create_family(&mut self, email: &str) -> Result<Family> {
        let email = email.trim().to_ascii_lowercase();
        let f = Family {
            id: ulid::Ulid::generate().to_string(),
            email,
            created_at: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64),
        };
        let tx = self.conn.transaction()?;
        let inserted = tx
            .execute(
                "INSERT INTO families(id,email,created_at) VALUES (?1,?2,?3)",
                rusqlite::params![f.id, f.email, f.created_at],
            )
            .and_then(|_| {
                tx.execute(
                    "INSERT INTO family_parents(family_id,email,added_at) VALUES (?1,?2,?3)",
                    rusqlite::params![f.id, f.email, f.created_at],
                )
            });
        match inserted {
            Ok(_) => {}
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                return Err(DbError::Taken(f.email));
            }
            Err(e) => return Err(e.into()),
        }
        let path = self.dir.join(crate::family_file_name(&f.id));
        if let Err(e) = family::open_family(&path) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        tx.commit()?;
        Ok(f)
    }

    /// Adds `email` as a co-parent of family `family_id`. An email already a parent of any family
    /// is `Taken`.
    pub fn add_parent(&mut self, family_id: &str, email: &str) -> Result<()> {
        valid_id(family_id)
            .then_some(())
            .ok_or_else(|| DbError::BadId(family_id.to_string()))?;
        let email = email.trim().to_ascii_lowercase();
        if !self
            .conn
            .query_row(
                "SELECT 1 FROM families WHERE id=?1",
                [family_id],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(DbError::Invalid(format!("no family {family_id}")));
        }
        let added_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        match self.conn.execute(
            "INSERT INTO family_parents(family_id,email,added_at) VALUES (?1,?2,?3)",
            rusqlite::params![family_id, email, added_at],
        ) {
            Ok(_) => Ok(()),
            Err(rusqlite::Error::SqliteFailure(e, _))
                if e.code == rusqlite::ErrorCode::ConstraintViolation =>
            {
                Err(DbError::Taken(email))
            }
            Err(e) => Err(e.into()),
        }
    }

    /// The parent emails of `family_id`, sorted.
    pub fn parents(&self, family_id: &str) -> Result<Vec<String>> {
        let mut s = self
            .conn
            .prepare("SELECT email FROM family_parents WHERE family_id=?1 ORDER BY email")?;
        let rows = s.query_map([family_id], |r| r.get(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    /// The family `email` is a parent of.
    pub fn lookup(&self, email: &str) -> Result<Option<Family>> {
        Ok(self
            .conn
            .query_row(
                "SELECT f.id,f.email,f.created_at FROM family_parents p
                 JOIN families f ON f.id=p.family_id WHERE p.email=?1",
                [email.trim().to_ascii_lowercase()],
                row,
            )
            .optional()?)
    }

    pub fn list(&self) -> Result<Vec<Family>> {
        let mut s = self
            .conn
            .prepare("SELECT id,email,created_at FROM families ORDER BY id")?;
        let rows = s.query_map([], row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn family_path(&self, id: &str) -> Result<PathBuf> {
        if !valid_id(id) {
            return Err(DbError::BadId(id.to_string()));
        }
        Ok(self.dir.join(crate::family_file_name(id)))
    }

    pub fn open_family(&self, id: &str) -> Result<Connection> {
        family::open_family(&self.family_path(id)?)
    }
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Family> {
    Ok(Family {
        id: r.get(0)?,
        email: r.get(1)?,
        created_at: r.get(2)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DbError;

    #[test]
    fn create_then_lookup_is_case_insensitive_and_files_are_per_family() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        let f = r.create_family("Mum@Example.com").unwrap();
        assert_eq!(r.lookup("mum@example.com").unwrap(), Some(f.clone()));
        assert!(r.lookup("other@example.com").unwrap().is_none());
        let path = r.family_path(&f.id).unwrap();
        assert!(path.exists());
        assert_eq!(
            path.file_name().unwrap(),
            format!("family-{}.sqlite", f.id).as_str()
        );
        assert_eq!(r.list().unwrap(), [f]);
    }

    #[test]
    fn a_second_family_for_the_same_email_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        r.create_family("a@b.c").unwrap();
        assert!(matches!(r.create_family("A@B.C"), Err(DbError::Taken(_))));
        assert_eq!(r.list().unwrap().len(), 1);
    }

    #[test]
    fn a_co_parent_resolves_to_the_same_family() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        let f = r.create_family("ryan@x.y").unwrap();
        r.add_parent(&f.id, " Kim@Gmail.com ").unwrap();
        assert_eq!(r.lookup("kim@gmail.com").unwrap(), Some(f.clone()));
        assert_eq!(r.lookup("ryan@x.y").unwrap(), Some(f.clone()));
        assert_eq!(r.parents(&f.id).unwrap(), ["kim@gmail.com", "ryan@x.y"]);
        assert_eq!(r.list().unwrap(), [f]);
    }

    #[test]
    fn a_parent_email_cannot_belong_to_two_families() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        let a = r.create_family("a@b.c").unwrap();
        let b = r.create_family("d@e.f").unwrap();
        assert!(matches!(
            r.add_parent(&b.id, "A@B.C"),
            Err(DbError::Taken(_))
        ));
        assert!(matches!(r.create_family("d@e.f"), Err(DbError::Taken(_))));
        assert!(matches!(
            r.add_parent("01ABC", "z@z.z"),
            Err(DbError::BadId(_))
        ));
        assert!(matches!(
            r.add_parent("01ARZ3NDEKTSV4RRFFQ69G5FAV", "z@z.z"),
            Err(DbError::Invalid(_))
        ));
        assert_eq!(r.parents(&a.id).unwrap(), ["a@b.c"]);
    }

    #[test]
    fn migrating_a_v1_registry_seeds_family_parents_from_families() {
        let d = tempfile::tempdir().unwrap();
        let c = Connection::open(d.path().join("registry.sqlite")).unwrap();
        c.execute_batch(MIGRATIONS[0]).unwrap();
        c.execute(
            "INSERT INTO families(id,email,created_at) VALUES ('01ARZ3NDEKTSV4RRFFQ69G5FAV','old@x.y',1)",
            [],
        )
        .unwrap();
        c.pragma_update(None, "user_version", 1).unwrap();
        drop(c);
        let r = Registry::open(d.path()).unwrap();
        let f = r.lookup("old@x.y").unwrap().unwrap();
        assert_eq!(f.id, "01ARZ3NDEKTSV4RRFFQ69G5FAV");
        assert_eq!(r.parents(&f.id).unwrap(), ["old@x.y"]);
    }

    #[test]
    fn two_families_get_distinct_files() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        let a = r.create_family("a@b.c").unwrap();
        let b = r.create_family("d@e.f").unwrap();
        assert_ne!(r.family_path(&a.id).unwrap(), r.family_path(&b.id).unwrap());
    }

    #[test]
    fn an_id_that_is_not_a_family_id_cannot_name_a_path() {
        let d = tempfile::tempdir().unwrap();
        let r = Registry::open(d.path()).unwrap();
        for bad in ["../x", "", "a/b", "01ABC"] {
            assert!(
                matches!(r.family_path(bad), Err(DbError::BadId(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn the_registry_refuses_a_newer_schema() {
        let d = tempfile::tempdir().unwrap();
        drop(Registry::open(d.path()).unwrap());
        let c = Connection::open(d.path().join("registry.sqlite")).unwrap();
        c.pragma_update(None, "user_version", REGISTRY_VERSION + 1)
            .unwrap();
        drop(c);
        assert!(matches!(
            Registry::open(d.path()),
            Err(DbError::NewerSchema { .. })
        ));
    }

    #[test]
    fn open_family_provisions_a_usable_family_database() {
        let d = tempfile::tempdir().unwrap();
        let mut r = Registry::open(d.path()).unwrap();
        let f = r.create_family("a@b.c").unwrap();
        let c = r.open_family(&f.id).unwrap();
        let n: i64 = c
            .query_row("SELECT count(*) FROM children", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }
}

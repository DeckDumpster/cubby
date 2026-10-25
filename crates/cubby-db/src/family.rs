use std::path::Path;
use std::time::Duration;

use rusqlite::Connection;

use crate::error::Result;
use crate::schema;

/// The family schema version this build understands: one migration per version.
pub const FAMILY_VERSION: i64 = MIGRATIONS.len() as i64;

const MIGRATIONS: [&str; 8] = [
    include_str!("family_v1.sql"),
    include_str!("family_v2.sql"),
    include_str!("family_v3.sql"),
    include_str!("family_v4.sql"),
    include_str!("family_v5.sql"),
    include_str!("family_v6.sql"),
    include_str!("family_v7.sql"),
    include_str!("family_v8.sql"),
];

/// Brings `conn` up to this build's family schema; refuses a newer one.
pub fn migrate_family(conn: &mut Connection) -> Result<()> {
    schema::migrate(conn, "family", &MIGRATIONS)
}

/// Opens (creating and migrating) a family database file.
pub fn open_family(path: &Path) -> Result<Connection> {
    let mut conn = Connection::open(path)?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(Duration::from_secs(5))?;
    migrate_family(&mut conn)?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mem() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_family(&mut c).unwrap();
        c
    }

    #[test]
    fn a_family_file_refuses_a_newer_schema() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.sqlite");
        drop(open_family(&p).unwrap());
        let c = Connection::open(&p).unwrap();
        c.pragma_update(None, "user_version", FAMILY_VERSION + 1)
            .unwrap();
        drop(c);
        assert!(matches!(
            open_family(&p),
            Err(crate::DbError::NewerSchema { .. })
        ));
    }

    #[test]
    fn the_rolling_window_column_is_gone_and_budgets_survive() {
        let mut c = Connection::open_in_memory().unwrap();
        for m in &MIGRATIONS[..6] {
            c.execute_batch(m).unwrap();
        }
        c.pragma_update(None, "user_version", 6).unwrap();
        c.execute_batch(
            "INSERT INTO children(id,name,avatar,tz) VALUES ('a','A','','UTC');
             INSERT INTO budgets(child_id,video_equivalents,window_s,minutes_cap) VALUES ('a',3,3600,30);",
        )
        .unwrap();
        migrate_family(&mut c).unwrap();
        let (v, m): (f64, i64) = c
            .query_row(
                "SELECT video_equivalents, minutes_cap FROM budgets",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((v, m), (3.0, 30));
        assert!(c.prepare("SELECT window_s FROM budgets").is_err());
    }

    #[test]
    fn a_family_file_opens_twice_and_stays_at_the_current_version() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f.sqlite");
        drop(open_family(&p).unwrap());
        let c = open_family(&p).unwrap();
        let v: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, FAMILY_VERSION);
    }

    fn search(c: &Connection, q: &str) -> Vec<String> {
        let mut st = c
            .prepare(
                "SELECT v.youtube_id FROM videos_fts f JOIN videos v ON v.rowid=f.rowid \
                 WHERE videos_fts MATCH ?1 ORDER BY v.youtube_id",
            )
            .unwrap();
        st.query_map([q], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn videos_are_indexed_on_insert_update_and_delete() {
        let c = mem();
        c.execute_batch(
            "INSERT INTO videos(youtube_id,title,channel_title,tags) VALUES ('v1','Golden hour','Sun TV','[\"calm\"]');",
        )
        .unwrap();
        assert_eq!(search(&c, "golden"), ["v1"]);
        assert_eq!(search(&c, "sun"), ["v1"]);
        assert_eq!(search(&c, "calm"), ["v1"]);
        c.execute("UPDATE videos SET title='Silver' WHERE youtube_id='v1'", [])
            .unwrap();
        assert!(search(&c, "golden").is_empty());
        assert_eq!(search(&c, "silver"), ["v1"]);
        c.execute("DELETE FROM videos WHERE youtube_id='v1'", [])
            .unwrap();
        assert!(search(&c, "silver").is_empty());
    }

    #[test]
    fn migration_rebuilds_the_index_of_an_existing_file() {
        let mut c = Connection::open_in_memory().unwrap();
        schema::migrate(&mut c, "family", &MIGRATIONS[..2]).unwrap();
        c.execute(
            "INSERT INTO videos(youtube_id,title) VALUES ('v1','Golden')",
            [],
        )
        .unwrap();
        assert!(search(&c, "golden").is_empty());
        migrate_family(&mut c).unwrap();
        assert_eq!(search(&c, "golden"), ["v1"]);
    }

    fn seed(c: &Connection) {
        c.execute_batch(
            "INSERT INTO children(id,name,avatar,tz) VALUES ('a','A','', 'UTC'),('b','B','', 'UTC');
             INSERT INTO videos(youtube_id,title) VALUES ('v1','one'),('v2','two');
             INSERT INTO allow_policies(id,kind,youtube_id,child_id,added_at) VALUES
               ('p1','video','v1',NULL,0),('p2','video','v2','a',0);",
        )
        .unwrap();
    }

    fn playable(c: &Connection, child: &str) -> Vec<String> {
        let mut s = c
            .prepare("SELECT youtube_id FROM playable WHERE child_id=?1 ORDER BY youtube_id")
            .unwrap();
        s.query_map([child], |r| r.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    fn allow(c: &Connection, vid: &str, child: Option<&str>, policy: &str, state: &str) {
        c.execute(
            "INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES (?1,?2,?3,?4,0)",
            rusqlite::params![vid, child, policy, state],
        )
        .unwrap();
    }

    #[test]
    fn allowed_by_policy_for_all_children() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        assert_eq!(playable(&c, "a"), ["v1"]);
        assert_eq!(playable(&c, "b"), ["v1"]);
    }

    #[test]
    fn approved_counts_as_allowed() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "approved");
        assert_eq!(playable(&c, "a"), ["v1"]);
    }

    #[test]
    fn nothing_allowed_means_nothing_playable() {
        let c = mem();
        seed(&c);
        assert!(playable(&c, "a").is_empty());
    }

    #[test]
    fn per_child_scope_reaches_only_that_child() {
        let c = mem();
        seed(&c);
        allow(&c, "v2", Some("a"), "p2", "auto-allowed");
        assert_eq!(playable(&c, "a"), ["v2"]);
        assert!(playable(&c, "b").is_empty());
    }

    #[test]
    fn a_block_for_all_children_beats_an_allow() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        allow(&c, "v1", None, "p1b", "blocked");
        assert!(playable(&c, "a").is_empty());
        assert!(playable(&c, "b").is_empty());
    }

    #[test]
    fn a_block_for_one_child_leaves_the_others() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        allow(&c, "v1", Some("a"), "p1", "blocked");
        assert!(playable(&c, "a").is_empty());
        assert_eq!(playable(&c, "b"), ["v1"]);
    }

    #[test]
    fn a_block_outlives_a_removed_policy() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        allow(&c, "v1", None, "p1", "blocked");
        c.execute("UPDATE allow_policies SET removed_at=5 WHERE id='p1'", [])
            .unwrap();
        assert!(playable(&c, "a").is_empty());
    }

    #[test]
    fn a_removed_policy_stops_allowing() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        c.execute("UPDATE allow_policies SET removed_at=5 WHERE id='p1'", [])
            .unwrap();
        assert!(playable(&c, "a").is_empty());
    }

    #[test]
    fn another_live_policy_keeps_the_video_allowed() {
        let c = mem();
        seed(&c);
        c.execute(
            "INSERT INTO allow_policies(id,kind,youtube_id,child_id,added_at) VALUES ('p3','channel','ch',NULL,0)",
            [],
        )
        .unwrap();
        allow(&c, "v1", None, "p1", "auto-allowed");
        allow(&c, "v1", None, "p3", "auto-allowed");
        c.execute("UPDATE allow_policies SET removed_at=5 WHERE id='p1'", [])
            .unwrap();
        assert_eq!(playable(&c, "a"), ["v1"]);
    }

    #[test]
    fn an_unavailable_video_is_not_playable() {
        let c = mem();
        seed(&c);
        allow(&c, "v1", None, "p1", "auto-allowed");
        c.execute(
            "UPDATE videos SET unavailable_at=9 WHERE youtube_id='v1'",
            [],
        )
        .unwrap();
        assert!(playable(&c, "a").is_empty());
    }

    #[test]
    fn a_video_with_no_metadata_row_is_not_playable() {
        let c = mem();
        seed(&c);
        allow(&c, "ghost", None, "p1", "auto-allowed");
        assert!(playable(&c, "a").is_empty());
    }
}

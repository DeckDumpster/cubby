//! Child-facing reads: the playable catalogue, search, and the child's watch history.
//!
//! Every query here reads the `playable(child_id, youtube_id)` view and nothing else decides
//! what a child may see.

use std::collections::HashSet;

use rusqlite::{Connection, params};

#[derive(Clone, Debug, PartialEq)]
pub struct VideoRow {
    pub youtube_id: String,
    pub title: String,
    pub channel_id: String,
    pub channel_title: String,
    pub tags: Vec<String>,
    pub duration_s: i64,
    pub thumb_path: Option<String>,
    /// Unix seconds the row was fetched; the closest upload-date signal the schema carries.
    pub published_at: i64,
}

const COLUMNS: &str = "v.youtube_id, v.title, v.channel_id, v.channel_title, v.tags, \
                       v.duration_s, v.thumb_path, v.fetched_at";

fn row(r: &rusqlite::Row) -> rusqlite::Result<VideoRow> {
    let tags: Option<String> = r.get(4)?;
    Ok(VideoRow {
        youtube_id: r.get(0)?,
        title: r.get(1)?,
        channel_id: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
        channel_title: r.get(3)?,
        tags: tags
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default(),
        duration_s: r.get::<_, Option<i64>>(5)?.unwrap_or(0),
        thumb_path: r.get(6)?,
        published_at: r.get::<_, Option<i64>>(7)?.unwrap_or(0),
    })
}

/// Every video the child may play, in id order.
pub fn playable_videos(conn: &Connection, child_id: &str) -> rusqlite::Result<Vec<VideoRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM playable p JOIN videos v ON v.youtube_id = p.youtube_id \
         WHERE p.child_id = ?1 ORDER BY v.youtube_id"
    );
    conn.prepare(&sql)?
        .query_map(params![child_id], row)?
        .collect()
}

/// Turns free text into an FTS5 query that cannot be a syntax error: each word is a quoted
/// term, the last one a prefix, all required. `None` when there is nothing to search for.
pub fn fts_query(q: &str) -> Option<String> {
    let words: Vec<&str> = q
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let (last, init) = words.split_last()?;
    let mut parts: Vec<String> = init.iter().map(|w| format!("\"{w}\"")).collect();
    parts.push(format!("\"{last}\"*"));
    Some(parts.join(" "))
}

/// FTS5 matches joined to `playable`, best bm25 first. A video that is not playable for the
/// child cannot appear, whatever it matches.
pub fn search(
    conn: &Connection,
    child_id: &str,
    q: &str,
    limit: usize,
) -> rusqlite::Result<Vec<VideoRow>> {
    let Some(fts) = fts_query(q) else {
        return Ok(Vec::new());
    };
    let sql = format!(
        "SELECT {COLUMNS} FROM videos_fts f \
         JOIN videos v ON v.rowid = f.rowid \
         JOIN playable p ON p.youtube_id = v.youtube_id AND p.child_id = ?1 \
         WHERE videos_fts MATCH ?2 ORDER BY bm25(videos_fts), v.youtube_id LIMIT ?3"
    );
    conn.prepare(&sql)?
        .query_map(params![child_id, fts, limit as i64], row)?
        .collect()
}

/// What the child has already watched, started or liked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct History {
    pub watched: HashSet<String>,
    /// Started but not finished (under 90% of the duration reached), most recent first.
    pub resumable: Vec<String>,
    pub liked_channels: HashSet<String>,
}

pub fn history(conn: &Connection, child_id: &str) -> rusqlite::Result<History> {
    let mut h = History::default();
    let mut st = conn.prepare(
        "SELECT g.youtube_id, g.max_position_s, v.duration_s FROM play_grants g \
         JOIN videos v ON v.youtube_id = g.youtube_id \
         WHERE g.child_id = ?1 AND g.watched_s > 0 ORDER BY g.granted_at DESC",
    )?;
    let rows = st.query_map(params![child_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<f64>>(1)?.unwrap_or(0.0),
            r.get::<_, Option<i64>>(2)?.unwrap_or(0),
        ))
    })?;
    for r in rows {
        let (id, pos, dur) = r?;
        h.watched.insert(id.clone());
        if pos < 0.9 * dur as f64 && !h.resumable.contains(&id) {
            h.resumable.push(id);
        }
    }
    let mut st = conn.prepare(
        "SELECT DISTINCT v.channel_id FROM requests r \
         JOIN videos v ON v.youtube_id = r.youtube_id \
         WHERE r.child_id = ?1 AND r.kind = 'like'",
    )?;
    for c in st.query_map(params![child_id], |r| r.get::<_, String>(0))? {
        h.liked_channels.insert(c?);
    }
    Ok(h)
}

/// Fixtures over the real family schema: children `kid` and `sib`, and a device `d` for `kid`.
pub mod fixture {
    use rusqlite::Connection;

    pub fn open() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        c.pragma_update(None, "foreign_keys", "ON").unwrap();
        crate::family::migrate_family(&mut c).unwrap();
        c.execute_batch(
            "INSERT INTO children VALUES('kid','Kid','','UTC'),('sib','Sib','','UTC');
             INSERT INTO devices(id,child_id,token_hash,paired_at) VALUES('d','kid',x'00',0);",
        )
        .unwrap();
        c
    }

    /// Adds a video, indexes it, and allows it for `child` (`approved`).
    pub fn add_video(c: &Connection, id: &str, title: &str, channel: &str, tags: &str, age: i64) {
        c.execute(
            "INSERT INTO videos VALUES(?1,?2,?3,?3,?4,300,NULL,1,?5,NULL)",
            rusqlite::params![id, title, channel, tags, 1_000_000 - age],
        )
        .unwrap();
    }

    pub fn allow(c: &Connection, id: &str, child: &str, state: &str) {
        let policy = format!("{id}-{child}-{state}");
        c.execute(
            "INSERT INTO allow_policies VALUES(?1,'video',?2,?3,0,NULL)",
            rusqlite::params![policy, id, child],
        )
        .unwrap();
        c.execute(
            "INSERT INTO allowed_videos VALUES(?1,?2,?3,?4,0)",
            rusqlite::params![id, child, policy, state],
        )
        .unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixture::*;
    use super::*;

    fn ids(v: Vec<VideoRow>) -> Vec<String> {
        v.into_iter().map(|r| r.youtube_id).collect()
    }

    #[test]
    fn fts_query_quotes_words_and_prefixes_the_last() {
        assert_eq!(fts_query("blue  tr").unwrap(), "\"blue\" \"tr\"*");
        assert_eq!(
            fts_query("a\" OR NOT (b").unwrap(),
            "\"a\" \"OR\" \"NOT\" \"b\"*"
        );
        assert_eq!(fts_query("  ?! "), None);
    }

    #[test]
    fn search_returns_only_playable_videos() {
        let c = open();
        add_video(&c, "ok", "Blue train", "Trains", "[]", 0);
        add_video(&c, "blocked", "Blue blocked train", "Trains", "[]", 0);
        add_video(&c, "unlisted", "Blue unlisted train", "Trains", "[]", 0);
        add_video(&c, "other", "Blue other train", "Trains", "[]", 0);
        allow(&c, "ok", "kid", "approved");
        allow(&c, "blocked", "kid", "approved");
        allow(&c, "blocked", "kid", "blocked");
        allow(&c, "other", "sibling", "approved");
        assert_eq!(ids(search(&c, "kid", "blue train", 10).unwrap()), ["ok"]);
        assert_eq!(ids(search(&c, "kid", "blu", 10).unwrap()), ["ok"]);
    }

    #[test]
    fn search_hostile_input_is_not_a_syntax_error() {
        let c = open();
        add_video(&c, "ok", "Blue train", "Trains", "[]", 0);
        allow(&c, "ok", "kid", "approved");
        for q in ["\"", "train AND", "* ^ (", "NEAR(", "title:x"] {
            assert!(search(&c, "kid", q, 10).is_ok(), "{q}");
        }
    }

    #[test]
    fn search_ranks_by_bm25() {
        let c = open();
        add_video(&c, "a", "train", "Misc", "[]", 0);
        add_video(&c, "b", "train train train", "Misc", "[]", 0);
        allow(&c, "a", "kid", "approved");
        allow(&c, "b", "kid", "approved");
        assert_eq!(ids(search(&c, "kid", "train", 10).unwrap()), ["b", "a"]);
    }

    #[test]
    fn history_reports_watched_resumable_and_liked() {
        let c = open();
        add_video(&c, "half", "t", "c1", "[]", 0);
        add_video(&c, "done", "t", "c2", "[]", 0);
        c.execute_batch(
            "INSERT INTO play_grants VALUES('g1','kid','d','half',1,1,100,100,0.3,NULL,NULL,NULL);
             INSERT INTO play_grants VALUES('g2','kid','d','done',2,2,295,295,1,NULL,NULL,NULL);
             INSERT INTO requests VALUES('r','kid','like','done',1,NULL,NULL);",
        )
        .unwrap();
        let h = history(&c, "kid").unwrap();
        assert_eq!(h.resumable, ["half"]);
        assert_eq!(h.watched.len(), 2);
        assert!(h.liked_channels.contains("c2"));
    }
}

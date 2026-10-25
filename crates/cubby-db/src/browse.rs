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

pub(crate) const COLUMNS: &str = "v.youtube_id, v.title, v.channel_id, v.channel_title, v.tags, \
                       v.duration_s, v.thumb_path, v.fetched_at";

pub(crate) fn row(r: &rusqlite::Row) -> rusqlite::Result<VideoRow> {
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

/// A channel that has at least one video the child may play.
#[derive(Clone, Debug, PartialEq)]
pub struct ChannelRow {
    pub channel_id: String,
    pub title: String,
    /// Thumbnail of the channel's newest playable video.
    pub thumb_path: Option<String>,
    pub published_at: i64,
}

/// The child's channels, newest activity first. Built from `playable` only, so a channel with
/// nothing the child may watch does not exist here.
pub fn channels(conn: &Connection, child_id: &str) -> rusqlite::Result<Vec<ChannelRow>> {
    let mut by_channel: Vec<ChannelRow> = Vec::new();
    for v in playable_videos(conn, child_id)? {
        if v.channel_id.is_empty() {
            continue;
        }
        match by_channel.iter_mut().find(|c| c.channel_id == v.channel_id) {
            Some(c) if v.published_at > c.published_at => {
                c.title = v.channel_title;
                c.thumb_path = v.thumb_path;
                c.published_at = v.published_at;
            }
            Some(_) => {}
            None => by_channel.push(ChannelRow {
                channel_id: v.channel_id,
                title: v.channel_title,
                thumb_path: v.thumb_path,
                published_at: v.published_at,
            }),
        }
    }
    by_channel.sort_by(|a, b| {
        b.published_at
            .cmp(&a.published_at)
            .then_with(|| a.channel_id.cmp(&b.channel_id))
    });
    Ok(by_channel)
}

/// The playable videos of one channel, newest first.
pub fn channel_videos(
    conn: &Connection,
    child_id: &str,
    channel_id: &str,
) -> rusqlite::Result<Vec<VideoRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM playable p JOIN videos v ON v.youtube_id = p.youtube_id \
         WHERE p.child_id = ?1 AND v.channel_id = ?2 \
         ORDER BY v.fetched_at DESC, v.youtube_id"
    );
    conn.prepare(&sql)?
        .query_map(params![child_id, channel_id], row)?
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChannelAsk {
    /// The channel is already allowed as a whole for this child (or all children).
    pub imported: bool,
    /// The child has an open more-from-channel request for it.
    pub requested: bool,
}

pub fn channel_ask(
    conn: &Connection,
    child_id: &str,
    channel_id: &str,
) -> rusqlite::Result<ChannelAsk> {
    let imported = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM allow_policies WHERE kind='channel' AND youtube_id=?2 \
           AND removed_at IS NULL AND (child_id IS NULL OR child_id=?1))",
        params![child_id, channel_id],
        |r| r.get(0),
    )?;
    let requested = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM requests q JOIN videos v ON v.youtube_id=q.youtube_id \
           WHERE q.child_id=?1 AND q.kind='more-from-channel' AND q.resolved_at IS NULL \
             AND v.channel_id=?2)",
        params![child_id, channel_id],
        |r| r.get(0),
    )?;
    Ok(ChannelAsk {
        imported,
        requested,
    })
}

/// Files a more-from-channel request for the child, keyed to the channel's newest playable
/// video. `None` when the child has no playable video there; otherwise the resulting state —
/// an already-open request or an imported channel files nothing new.
pub fn request_more_from_channel(
    conn: &Connection,
    child_id: &str,
    channel_id: &str,
    now: i64,
) -> rusqlite::Result<Option<ChannelAsk>> {
    let Some(newest) = channel_videos(conn, child_id, channel_id)?
        .into_iter()
        .next()
    else {
        return Ok(None);
    };
    let state = channel_ask(conn, child_id, channel_id)?;
    if state.imported || state.requested {
        return Ok(Some(state));
    }
    conn.execute(
        "INSERT INTO requests(id,child_id,kind,youtube_id,created_at) \
         VALUES (?1,?2,'more-from-channel',?3,?4)",
        params![
            ulid::Ulid::generate().to_string(),
            child_id,
            newest.youtube_id,
            now
        ],
    )?;
    Ok(Some(ChannelAsk {
        imported: false,
        requested: true,
    }))
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

/// A play shorter than this never really started (a refusal, an instant bounce).
pub const MIN_WATCHED_S: f64 = 5.0;

/// The child's watched videos, one per video at its latest real start, newest first. Reads
/// `playable`, so a video disallowed since is filtered out, not deleted from the history.
pub fn recent(
    conn: &Connection,
    child_id: &str,
    limit: usize,
    offset: usize,
) -> rusqlite::Result<Vec<VideoRow>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM \
           (SELECT youtube_id, MAX(granted_at) AS at FROM play_grants \
            WHERE child_id = ?1 AND watched_s >= ?2 GROUP BY youtube_id) g \
         JOIN videos v ON v.youtube_id = g.youtube_id \
         JOIN playable p ON p.youtube_id = g.youtube_id AND p.child_id = ?1 \
         ORDER BY g.at DESC, v.youtube_id LIMIT ?3 OFFSET ?4"
    );
    conn.prepare(&sql)?
        .query_map(
            params![child_id, MIN_WATCHED_S, limit as i64, offset as i64],
            row,
        )?
        .collect()
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
    fn channels_list_only_allowed_videos_including_one_offs() {
        let c = open();
        add_video(&c, "a1", "t", "Alpha", "[]", 5);
        add_video(&c, "a2", "t", "Alpha", "[]", 0);
        add_video(&c, "hidden", "t", "Alpha", "[]", 0);
        add_video(&c, "b1", "t", "Beta", "[]", 9);
        add_video(&c, "g1", "t", "Gamma", "[]", 0);
        allow(&c, "a1", "kid", "approved");
        allow(&c, "a2", "kid", "auto-allowed");
        allow(&c, "b1", "kid", "approved");
        allow(&c, "g1", "sib", "approved");
        let ch = channels(&c, "kid").unwrap();
        assert_eq!(
            ch.iter().map(|c| c.channel_id.as_str()).collect::<Vec<_>>(),
            ["Alpha", "Beta"]
        );
        assert_eq!(
            ids(channel_videos(&c, "kid", "Alpha").unwrap()),
            ["a2", "a1"]
        );
        assert!(channel_videos(&c, "kid", "Gamma").unwrap().is_empty());
    }

    #[test]
    fn requesting_more_files_one_open_request_and_not_for_imported_channels() {
        let c = open();
        add_video(&c, "a1", "t", "Alpha", "[]", 5);
        add_video(&c, "a2", "t", "Alpha", "[]", 0);
        allow(&c, "a1", "kid", "approved");
        assert_eq!(
            request_more_from_channel(&c, "kid", "Alpha", 7).unwrap(),
            Some(ChannelAsk {
                imported: false,
                requested: true
            })
        );
        request_more_from_channel(&c, "kid", "Alpha", 8).unwrap();
        let n: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM requests WHERE kind='more-from-channel'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 1);
        assert!(!channel_ask(&c, "sib", "Alpha").unwrap().requested);
        assert_eq!(
            request_more_from_channel(&c, "sib", "Alpha", 9).unwrap(),
            None
        );

        c.execute_batch("INSERT INTO allow_policies VALUES('cp','channel','Alpha','kid',0,NULL);")
            .unwrap();
        let st = channel_ask(&c, "kid", "Alpha").unwrap();
        assert!(st.imported);
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

    #[test]
    fn recent_is_newest_first_deduped_started_playable_and_own() {
        let c = open();
        for id in ["old", "mid", "new", "bounce", "gone", "theirs"] {
            add_video(&c, id, "t", "c", "[]", 0);
            allow(&c, id, "kid", "approved");
        }
        allow(&c, "gone", "kid", "blocked");
        c.execute_batch(
            "INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,watched_s) VALUES
               ('1','kid','d','old',10,60),('2','kid','d','mid',20,60),('3','kid','d','new',30,60),
               ('4','kid','d','old',40,60),('5','kid','d','new',25,60),
               ('6','kid','d','bounce',50,0),('7','kid','d','gone',60,60),
               ('8','sib','d','theirs',70,60);",
        )
        .unwrap();
        let all = |o| ids(recent(&c, "kid", 10, o).unwrap());
        assert_eq!(all(0), ["old", "new", "mid"]);
        assert_eq!(ids(recent(&c, "kid", 2, 1).unwrap()), ["new", "mid"]);
        assert!(ids(recent(&c, "nobody", 10, 0).unwrap()).is_empty());
    }
}

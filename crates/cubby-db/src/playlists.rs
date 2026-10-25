//! A child's playlists. Today there is one, the built-in `watch_later`; every function is keyed
//! by kind so a future named playlist is a new id, not a new shape.
//!
//! Membership is a pointer and nothing here decides what a child may see: `list` joins
//! `playable`, so a video the parent later disallows is filtered out, not deleted.

use rusqlite::{Connection, OptionalExtension, params};

use crate::browse::{COLUMNS, VideoRow, row};

pub const WATCH_LATER: &str = "watch_later";

/// The kinds a playlist may have, with the name each is created under.
fn kind_name(kind: &str) -> Option<&'static str> {
    match kind {
        WATCH_LATER => Some("Watch later"),
        _ => None,
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PlaylistError {
    UnknownKind,
    /// The video is not playable for this child, so it cannot be saved.
    NotAllowed,
    Db(String),
}

impl From<rusqlite::Error> for PlaylistError {
    fn from(e: rusqlite::Error) -> Self {
        PlaylistError::Db(e.to_string())
    }
}

fn existing(conn: &Connection, child: &str, kind: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row(
        "SELECT id FROM playlists WHERE child_id=?1 AND kind=?2",
        params![child, kind],
        |r| r.get(0),
    )
    .optional()
}

/// The child's playlist of `kind`, created on first use.
pub fn ensure(
    conn: &Connection,
    child: &str,
    kind: &str,
    now: i64,
) -> Result<String, PlaylistError> {
    let name = kind_name(kind).ok_or(PlaylistError::UnknownKind)?;
    conn.execute(
        "INSERT OR IGNORE INTO playlists(id,child_id,kind,name,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![ulid::Ulid::generate().to_string(), child, kind, name, now],
    )?;
    Ok(existing(conn, child, kind)?.expect("just ensured"))
}

/// Saves `youtube_id`; saving it again changes nothing. Only a playable video can be saved.
pub fn add(
    conn: &Connection,
    child: &str,
    kind: &str,
    youtube_id: &str,
    now: i64,
) -> Result<(), PlaylistError> {
    kind_name(kind).ok_or(PlaylistError::UnknownKind)?;
    let ok: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM playable WHERE child_id=?1 AND youtube_id=?2)",
        params![child, youtube_id],
        |r| r.get(0),
    )?;
    if !ok {
        return Err(PlaylistError::NotAllowed);
    }
    let pl = ensure(conn, child, kind, now)?;
    conn.execute(
        "INSERT OR IGNORE INTO playlist_items(playlist_id,youtube_id,added_at) VALUES (?1,?2,?3)",
        params![pl, youtube_id, now],
    )?;
    Ok(())
}

/// Removes `youtube_id`; removing one that is not there changes nothing.
pub fn remove(
    conn: &Connection,
    child: &str,
    kind: &str,
    youtube_id: &str,
) -> Result<(), PlaylistError> {
    kind_name(kind).ok_or(PlaylistError::UnknownKind)?;
    conn.execute(
        "DELETE FROM playlist_items WHERE youtube_id=?2 AND playlist_id IN \
         (SELECT id FROM playlists WHERE child_id=?1 AND kind=?3)",
        params![child, youtube_id, kind],
    )?;
    Ok(())
}

/// The playable videos on the list, newest saved first.
pub fn list(conn: &Connection, child: &str, kind: &str) -> Result<Vec<VideoRow>, PlaylistError> {
    kind_name(kind).ok_or(PlaylistError::UnknownKind)?;
    let sql = format!(
        "SELECT {COLUMNS} FROM playlist_items i \
         JOIN playlists l ON l.id = i.playlist_id AND l.child_id = ?1 AND l.kind = ?2 \
         JOIN playable p ON p.child_id = l.child_id AND p.youtube_id = i.youtube_id \
         JOIN videos v ON v.youtube_id = i.youtube_id \
         ORDER BY i.added_at DESC, v.youtube_id"
    );
    Ok(conn
        .prepare(&sql)?
        .query_map(params![child, kind], row)?
        .collect::<Result<_, _>>()?)
}

/// Every video id on any of the child's lists of `kind`, for marking tiles as saved. Not
/// filtered by `playable`: a tile is only ever shown for a playable video anyway.
pub fn saved_ids(
    conn: &Connection,
    child: &str,
    kind: &str,
) -> rusqlite::Result<std::collections::HashSet<String>> {
    conn.prepare(
        "SELECT i.youtube_id FROM playlist_items i \
         JOIN playlists l ON l.id = i.playlist_id WHERE l.child_id=?1 AND l.kind=?2",
    )?
    .query_map(params![child, kind], |r| r.get(0))?
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browse::fixture::*;

    fn ids(c: &Connection, child: &str) -> Vec<String> {
        list(c, child, WATCH_LATER)
            .unwrap()
            .into_iter()
            .map(|v| v.youtube_id)
            .collect()
    }

    fn setup() -> Connection {
        let c = open();
        for id in ["a", "b", "c"] {
            add_video(&c, id, id, "ch", "[]", 0);
            allow(&c, id, "kid", "approved");
        }
        c
    }

    #[test]
    fn add_remove_and_a_repeated_add_are_idempotent() {
        let c = setup();
        add(&c, "kid", WATCH_LATER, "a", 10).unwrap();
        add(&c, "kid", WATCH_LATER, "a", 20).unwrap();
        add(&c, "kid", WATCH_LATER, "b", 30).unwrap();
        assert_eq!(ids(&c, "kid"), ["b", "a"], "newest saved first");
        let n: i64 = c
            .query_row("SELECT COUNT(*) FROM playlists", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "one watch_later list per child, created lazily");
        remove(&c, "kid", WATCH_LATER, "a").unwrap();
        remove(&c, "kid", WATCH_LATER, "a").unwrap();
        assert_eq!(ids(&c, "kid"), ["b"]);
    }

    #[test]
    fn a_disallowed_video_cannot_be_saved() {
        let c = setup();
        add_video(&c, "x", "x", "ch", "[]", 0);
        assert_eq!(
            add(&c, "kid", WATCH_LATER, "x", 1),
            Err(PlaylistError::NotAllowed)
        );
        assert_eq!(
            add(&c, "kid", WATCH_LATER, "nope", 1),
            Err(PlaylistError::NotAllowed)
        );
        assert!(ids(&c, "kid").is_empty());
    }

    #[test]
    fn a_video_disallowed_later_is_hidden_not_deleted() {
        let c = setup();
        add(&c, "kid", WATCH_LATER, "a", 1).unwrap();
        add(&c, "kid", WATCH_LATER, "b", 2).unwrap();
        allow(&c, "a", "kid", "blocked");
        assert_eq!(ids(&c, "kid"), ["b"]);
        c.execute("DELETE FROM allowed_videos WHERE state='blocked'", [])
            .unwrap();
        assert_eq!(ids(&c, "kid"), ["b", "a"], "still saved once allowed again");
    }

    #[test]
    fn another_childs_list_is_not_visible() {
        let c = setup();
        allow(&c, "a", "sib", "approved");
        add(&c, "kid", WATCH_LATER, "a", 1).unwrap();
        assert!(ids(&c, "sib").is_empty());
        assert!(saved_ids(&c, "sib", WATCH_LATER).unwrap().is_empty());
        remove(&c, "sib", WATCH_LATER, "a").unwrap();
        assert_eq!(ids(&c, "kid"), ["a"], "a sibling cannot remove it");
    }

    #[test]
    fn only_the_built_in_kind_exists() {
        let c = setup();
        assert_eq!(
            add(&c, "kid", "favourites", "a", 1),
            Err(PlaylistError::UnknownKind)
        );
        assert_eq!(
            list(&c, "kid", "favourites"),
            Err(PlaylistError::UnknownKind)
        );
    }
}

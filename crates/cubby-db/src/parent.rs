//! Parent-side reads and writes over a family database: the allow-list, the review list,
//! children, budgets, devices, history and the requests inbox.

use rusqlite::{Connection, OptionalExtension, params};

use crate::error::{DbError, Result};

/// The policy id a block row carries: a block belongs to no allow policy, so it outlives them.
const BLOCK_POLICY: &str = "block";

#[derive(Debug, Clone, PartialEq)]
pub struct Policy {
    pub id: String,
    pub kind: String,
    pub youtube_id: String,
    pub child_id: Option<String>,
    pub added_at: i64,
    /// The video or channel title when its metadata is known, else empty.
    pub title: String,
    /// True until the worker (or the allow request) has ingested what the policy names: the
    /// video's metadata, or any video a channel or playlist expands to.
    pub pending: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AutoItem {
    pub youtube_id: String,
    pub title: String,
    pub channel_id: Option<String>,
    pub channel_title: String,
    pub policy_id: String,
    pub policy_kind: String,
    pub policy_youtube_id: String,
    pub child_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Budget {
    pub video_equivalents: Option<f64>,
    pub minutes_cap: Option<i64>,
    /// Minutes of watching per video equivalent; `None` is the default.
    pub ve_minutes: Option<f64>,
    pub cutoff_mid_play: bool,
    pub daily_windows: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Grant {
    pub id: String,
    pub youtube_id: String,
    pub title: String,
    pub granted_at: i64,
    pub watched_s: f64,
    pub charge: f64,
    pub ended_at: Option<i64>,
    pub end_reason: Option<String>,
    pub thumb: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceRow {
    pub id: String,
    pub child_id: String,
    pub label: String,
    pub paired_at: i64,
    pub last_seen: Option<i64>,
    pub revoked_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChildRow {
    pub id: String,
    pub name: String,
    pub avatar: String,
    pub tz: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRequest {
    pub id: String,
    pub child_id: String,
    pub child_name: String,
    pub kind: String,
    pub youtube_id: String,
    pub title: String,
    pub channel_id: Option<String>,
    pub channel_title: String,
    pub created_at: i64,
}

/// Allows `youtube_id` of `kind` for `child` (or everyone): one live policy per (kind, id,
/// scope), and every video already known to belong to it is allowed straight away. A channel's
/// not-yet-known videos and a playlist's members arrive through the worker.
pub fn add_policy(
    conn: &Connection,
    kind: &str,
    youtube_id: &str,
    child: Option<&str>,
    now: i64,
) -> Result<String> {
    add_policy_labelled(conn, kind, youtube_id, child, now, None)
}

/// [`add_policy`] that also keeps `label` (what the parent typed, e.g. an `@handle`) to show
/// until the policy's own videos supply a title.
pub fn add_policy_labelled(
    conn: &Connection,
    kind: &str,
    youtube_id: &str,
    child: Option<&str>,
    now: i64,
    label: Option<&str>,
) -> Result<String> {
    if !matches!(kind, "video" | "channel" | "playlist") {
        return Err(DbError::Invalid(format!("unknown policy kind {kind:?}")));
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM allow_policies WHERE kind=?1 AND youtube_id=?2 AND child_id IS ?3
             AND removed_at IS NULL",
            params![kind, youtube_id, child],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(id);
    }
    let id = ulid::Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO allow_policies(id,kind,youtube_id,child_id,added_at) VALUES (?1,?2,?3,?4,?5)",
        params![id, kind, youtube_id, child, now],
    )?;
    if let Some(label) = label {
        conn.execute(
            "INSERT INTO policy_label(policy_id,label) VALUES (?1,?2)",
            params![id, label],
        )?;
    }
    match kind {
        "video" => {
            conn.execute(
                "INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at)
                 VALUES (?1,?2,?3,'approved',?4)",
                params![youtube_id, child, id, now],
            )?;
        }
        "channel" => {
            conn.execute(
                "INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at)
                 SELECT youtube_id, ?2, ?3, 'auto-allowed', ?4 FROM videos WHERE channel_id=?1",
                params![youtube_id, child, id, now],
            )?;
        }
        _ => {}
    }
    Ok(id)
}

/// Removes a policy; its videos stop being allowed by it (blocks stay). False if it was not live.
pub fn remove_policy(conn: &Connection, id: &str, now: i64) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE allow_policies SET removed_at=?2 WHERE id=?1 AND removed_at IS NULL",
        params![id, now],
    )? > 0)
}

/// Live channel policies still stored under an `@handle`: the Data API cannot expand those.
pub fn handle_policies(conn: &Connection) -> Result<Vec<(String, String)>> {
    let mut s = conn.prepare(
        "SELECT id, youtube_id FROM allow_policies
         WHERE kind='channel' AND youtube_id LIKE '@%' AND removed_at IS NULL
         ORDER BY added_at, id",
    )?;
    let rows = s.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Re-keys the `@handle` channel policy `id` to the channel's `UC` id, keeping the handle as its
/// label. When the same scope already allows that channel, the handle policy is removed instead.
pub fn resolve_handle_policy(
    conn: &Connection,
    id: &str,
    channel_id: &str,
    label: &str,
    now: i64,
) -> Result<()> {
    let dup: bool = conn.query_row(
        "SELECT EXISTS (SELECT 1 FROM allow_policies o, allow_policies p
           WHERE p.id=?1 AND o.id<>p.id AND o.kind='channel' AND o.youtube_id=?2
             AND o.child_id IS p.child_id AND o.removed_at IS NULL)",
        params![id, channel_id],
        |r| r.get(0),
    )?;
    if dup {
        conn.execute(
            "UPDATE allow_policies SET removed_at=?2 WHERE id=?1",
            params![id, now],
        )?;
        return Ok(());
    }
    conn.execute(
        "UPDATE allow_policies SET youtube_id=?2 WHERE id=?1",
        params![id, channel_id],
    )?;
    conn.execute(
        "INSERT OR REPLACE INTO policy_label(policy_id,label) VALUES (?1,?2)",
        params![id, label],
    )?;
    Ok(())
}

pub fn list_policies(conn: &Connection) -> Result<Vec<Policy>> {
    let mut s = conn.prepare(
        "SELECT p.id, p.kind, p.youtube_id, p.child_id, p.added_at,
            COALESCE(CASE p.kind
              WHEN 'video' THEN (SELECT title FROM videos WHERE youtube_id=p.youtube_id)
              WHEN 'channel' THEN (SELECT channel_title FROM videos WHERE channel_id=p.youtube_id LIMIT 1)
            END, (SELECT label FROM policy_label WHERE policy_id=p.id), ''),
            CASE p.kind
              WHEN 'video' THEN NOT EXISTS (SELECT 1 FROM videos WHERE youtube_id = p.youtube_id)
              ELSE NOT EXISTS (SELECT 1 FROM allowed_videos a WHERE a.policy_id = p.id)
            END
         FROM allow_policies p WHERE p.removed_at IS NULL ORDER BY p.added_at, p.id",
    )?;
    let rows = s.query_map([], |r| {
        Ok(Policy {
            id: r.get(0)?,
            kind: r.get(1)?,
            youtube_id: r.get(2)?,
            child_id: r.get(3)?,
            added_at: r.get(4)?,
            title: r.get(5)?,
            pending: r.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Blocks one video for `child` (or everyone), whatever allows it.
pub fn block_video(
    conn: &Connection,
    youtube_id: &str,
    child: Option<&str>,
    now: i64,
) -> Result<()> {
    unblock_video(conn, youtube_id, child)?;
    conn.execute(
        "INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at)
         VALUES (?1,?2,?3,'blocked',?4)",
        params![youtube_id, child, BLOCK_POLICY, now],
    )?;
    Ok(())
}

pub fn unblock_video(conn: &Connection, youtube_id: &str, child: Option<&str>) -> Result<()> {
    conn.execute(
        "DELETE FROM allowed_videos WHERE youtube_id=?1 AND child_id IS ?2 AND state='blocked'",
        params![youtube_id, child],
    )?;
    Ok(())
}

/// Marks a video the parent has looked at, so it leaves the review list and stays playable.
pub fn approve_video(conn: &Connection, youtube_id: &str, now: i64) -> Result<()> {
    conn.execute(
        "UPDATE allowed_videos SET state='approved', changed_at=?2
         WHERE youtube_id=?1 AND state='auto-allowed'",
        params![youtube_id, now],
    )?;
    Ok(())
}

/// What a channel or playlist policy lets through. Every field is optional; an all-default value
/// is "no rules". A video is playable through the policy only if it passes every rule set; the
/// `rule_failed` view is the one definition of "passes", read by `playable` and the parent lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Rules {
    /// Drop Shorts: a video of [`SHORT_MAX_S`] seconds or less.
    pub exclude_shorts: bool,
    pub min_s: Option<i64>,
    pub max_s: Option<i64>,
    /// The title must contain at least one of these (case-insensitive).
    pub title_has: Vec<String>,
    /// The title must contain none of these (case-insensitive).
    pub title_lacks: Vec<String>,
}

/// The longest a video can be and count as a Short. The Data API carries no Shorts flag, no
/// aspect ratio and no URL form, so duration is the only signal in the metadata Cubby fetches.
/// Keep in step with `family_v6.sql`.
pub const SHORT_MAX_S: i64 = 60;

/// Strips one leading/trailing quote pair, straight or curly: `"how to"` is the phrase `how to`.
fn unquote(s: &str) -> &str {
    for (open, close) in [('"', '"'), ('\u{201c}', '\u{201d}'), ('\'', '\'')] {
        if let Some(inner) = s.strip_prefix(open).and_then(|r| r.strip_suffix(close)) {
            return inner;
        }
    }
    s
}

impl Rules {
    pub fn is_empty(&self) -> bool {
        *self == Rules::default()
    }

    /// Lowercased, trimmed, de-duplicated phrases with one surrounding quote pair removed;
    /// blanks dropped. A phrase is matched whole, spaces and commas included.
    pub fn normalised(&self) -> Rules {
        let words = |w: &[String]| {
            let mut out: Vec<String> = w
                .iter()
                .map(|x| unquote(x.trim()).trim().to_lowercase())
                .filter(|x| !x.is_empty())
                .collect();
            out.sort();
            out.dedup();
            out
        };
        Rules {
            exclude_shorts: self.exclude_shorts,
            min_s: self.min_s,
            max_s: self.max_s,
            title_has: words(&self.title_has),
            title_lacks: words(&self.title_lacks),
        }
    }
}

/// Saves the rules of a live channel or playlist policy (an empty value clears them). False when
/// there is no such policy or it is a single-video policy, which has nothing to filter.
pub fn set_rules(conn: &Connection, policy: &str, rules: &Rules) -> Result<bool> {
    let kind: Option<String> = conn
        .query_row(
            "SELECT kind FROM allow_policies WHERE id=?1 AND removed_at IS NULL",
            [policy],
            |r| r.get(0),
        )
        .optional()?;
    if !matches!(kind.as_deref(), Some("channel" | "playlist")) {
        return Ok(false);
    }
    if let (Some(lo), Some(hi)) = (rules.min_s, rules.max_s)
        && lo > hi
    {
        return Err(DbError::Invalid(
            "the minimum duration is above the maximum".to_string(),
        ));
    }
    if rules.min_s.is_some_and(|m| m < 0) || rules.max_s.is_some_and(|m| m < 0) {
        return Err(DbError::Invalid(
            "a duration cannot be negative".to_string(),
        ));
    }
    let r = rules.normalised();
    conn.execute_batch("SAVEPOINT set_rules")?;
    let saved = (|| -> Result<()> {
        conn.execute("DELETE FROM policy_rules WHERE policy_id=?1", [policy])?;
        conn.execute("DELETE FROM policy_rule_words WHERE policy_id=?1", [policy])?;
        if !r.is_empty() {
            conn.execute(
            "INSERT INTO policy_rules(policy_id,exclude_shorts,min_s,max_s) VALUES (?1,?2,?3,?4)",
            params![policy, r.exclude_shorts, r.min_s, r.max_s],
        )?;
            for (mode, words) in [("has", &r.title_has), ("lacks", &r.title_lacks)] {
                for w in words {
                    conn.execute(
                        "INSERT INTO policy_rule_words(policy_id,mode,word) VALUES (?1,?2,?3)",
                        params![policy, mode, w],
                    )?;
                }
            }
        }
        Ok(())
    })();
    if saved.is_err() {
        conn.execute_batch("ROLLBACK TO set_rules")?;
    }
    conn.execute_batch("RELEASE set_rules")?;
    saved?;
    Ok(true)
}

pub fn get_rules(conn: &Connection, policy: &str) -> Result<Rules> {
    let mut rules = conn
        .query_row(
            "SELECT exclude_shorts,min_s,max_s FROM policy_rules WHERE policy_id=?1",
            [policy],
            |r| {
                Ok(Rules {
                    exclude_shorts: r.get(0)?,
                    min_s: r.get(1)?,
                    max_s: r.get(2)?,
                    ..Rules::default()
                })
            },
        )
        .optional()?
        .unwrap_or_default();
    let mut s =
        conn.prepare("SELECT mode,word FROM policy_rule_words WHERE policy_id=?1 ORDER BY word")?;
    for row in s.query_map([policy], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (mode, word) = row?;
        if mode == "has" {
            rules.title_has.push(word);
        } else {
            rules.title_lacks.push(word);
        }
    }
    Ok(rules)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExcludedVideo {
    pub youtube_id: String,
    pub title: String,
    pub duration_s: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RuleCounts {
    pub allowed: i64,
    pub excluded: i64,
    /// Up to the requested number of excluded videos, longest title-sorted first page.
    pub excluded_videos: Vec<ExcludedVideo>,
}

/// How many of a channel or playlist policy's videos the policy's saved rules allow and exclude,
/// and the first `limit` excluded ones. Blocked and unavailable videos are in neither count.
pub fn rule_counts(conn: &Connection, policy: &str, limit: usize) -> Result<RuleCounts> {
    let universe = "SELECT DISTINCT v.youtube_id AS youtube_id, v.title AS title, v.duration_s AS d
         FROM allowed_videos a JOIN videos v ON v.youtube_id=a.youtube_id
         WHERE a.policy_id=?1 AND a.state IN ('auto-allowed','approved')
           AND v.unavailable_at IS NULL
           AND NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=a.youtube_id
                AND b.state='blocked' AND (b.child_id IS NULL OR b.child_id IS a.child_id))";
    let excluded_where = "EXISTS (SELECT 1 FROM rule_failed f WHERE f.policy_id=?1
                AND f.youtube_id=u.youtube_id)";
    let (total, excluded): (i64, i64) = conn.query_row(
        &format!(
            "WITH u AS ({universe}) SELECT COUNT(*), COALESCE(SUM({excluded_where}),0) FROM u"
        ),
        [policy],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?;
    let mut s = conn.prepare(&format!(
        "WITH u AS ({universe}) SELECT youtube_id, title, d FROM u WHERE {excluded_where}
         ORDER BY title COLLATE NOCASE, youtube_id LIMIT ?2"
    ))?;
    let excluded_videos = s
        .query_map(params![policy, limit as i64], |r| {
            Ok(ExcludedVideo {
                youtube_id: r.get(0)?,
                title: r.get(1)?,
                duration_s: r.get(2)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok(RuleCounts {
        allowed: total - excluded,
        excluded,
        excluded_videos,
    })
}

/// [`rule_counts`] as if `rules` were saved, without saving them: the rules are written, counted
/// through the same view that enforces them, and rolled back. None when the policy takes no rules.
pub fn preview_rules(
    conn: &Connection,
    policy: &str,
    rules: &Rules,
    limit: usize,
) -> Result<Option<RuleCounts>> {
    conn.execute_batch("SAVEPOINT rules_preview")?;
    let out = set_rules(conn, policy, rules).and_then(|ok| {
        if ok {
            rule_counts(conn, policy, limit).map(Some)
        } else {
            Ok(None)
        }
    });
    conn.execute_batch("ROLLBACK TO rules_preview; RELEASE rules_preview")?;
    out
}

/// "Added by your policies": auto-allowed videos still allowed and not yet approved, with the
/// policy that let them in. `channel` filters by channel id.
pub fn auto_allowed(conn: &Connection, channel: Option<&str>) -> Result<Vec<AutoItem>> {
    let mut s = conn.prepare(
        "SELECT v.youtube_id, v.title, v.channel_id, v.channel_title,
                p.id, p.kind, p.youtube_id, a.child_id
         FROM allowed_videos a
         JOIN allow_policies p ON p.id=a.policy_id AND p.removed_at IS NULL
         JOIN videos v ON v.youtube_id=a.youtube_id AND v.unavailable_at IS NULL
         WHERE a.state='auto-allowed'
           AND (?1 IS NULL OR v.channel_id=?1)
           AND NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=a.youtube_id
                AND b.child_id IS a.child_id AND b.state='approved')
           AND NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=a.youtube_id
                AND b.state='blocked' AND (b.child_id IS NULL OR b.child_id IS a.child_id))
           AND NOT EXISTS (SELECT 1 FROM rule_failed f WHERE f.policy_id=a.policy_id
                AND f.youtube_id=a.youtube_id)
         GROUP BY v.youtube_id, a.child_id
         ORDER BY a.changed_at DESC, v.youtube_id",
    )?;
    let rows = s.query_map([channel], |r| {
        Ok(AutoItem {
            youtube_id: r.get(0)?,
            title: r.get(1)?,
            channel_id: r.get(2)?,
            channel_title: r.get(3)?,
            policy_id: r.get(4)?,
            policy_kind: r.get(5)?,
            policy_youtube_id: r.get(6)?,
            child_id: r.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[derive(Debug, Clone, Default)]
pub struct LibraryFilter {
    pub review: bool,
    /// Free text, matched with the FTS index over title, channel and tags.
    pub q: Option<String>,
    /// Scope the list and the play totals to one child.
    pub child: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryGroup {
    /// The channel id, `""` for videos with no known channel, or [`DIRECT_GROUP`].
    pub key: String,
    pub title: String,
    pub count: i64,
    pub playlists: Vec<LibrarySource>,
}

/// One allow policy that brought videos into a group.
#[derive(Debug, Clone, PartialEq)]
pub struct LibrarySource {
    pub policy_id: String,
    pub kind: String,
    pub youtube_id: String,
    pub count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LibraryVideo {
    pub youtube_id: String,
    pub title: String,
    pub channel_title: String,
    pub policy_id: String,
    pub policy_kind: String,
    pub policy_youtube_id: String,
    pub child_id: Option<String>,
    pub plays: i64,
    pub watched_s: f64,
    pub duration_s: Option<i64>,
}

/// The group holding videos allowed one by one.
pub const DIRECT_GROUP: &str = "direct";

/// Every (video, policy) pair the list shows, with the filter's FTS match applied. `?1` review,
/// `?2` child scope, `?3` FTS query: all optional.
const LIBRARY_ENTRIES: &str = "WITH e AS (
    SELECT v.youtube_id AS youtube_id, v.title AS title, v.channel_title AS channel_title,
           CASE WHEN p.kind='video' THEN 'direct' ELSE COALESCE(v.channel_id,'') END AS gkey,
           p.id AS pid, p.kind AS pkind, p.youtube_id AS pyt, a.child_id AS child_id,
           a.changed_at AS changed_at, v.duration_s AS duration_s
    FROM allowed_videos a
    JOIN allow_policies p ON p.id=a.policy_id AND p.removed_at IS NULL
    JOIN videos v ON v.youtube_id=a.youtube_id AND v.unavailable_at IS NULL
    WHERE (a.state='auto-allowed' OR (?1=0 AND a.state='approved'))
      AND (?2 IS NULL OR a.child_id IS NULL OR a.child_id=?2)
      AND (?1=0 OR NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=a.youtube_id
                AND b.child_id IS a.child_id AND b.state='approved'))
      AND NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=a.youtube_id
                AND b.state='blocked' AND (b.child_id IS NULL OR b.child_id IS a.child_id))
      AND NOT EXISTS (SELECT 1 FROM rule_failed f WHERE f.policy_id=a.policy_id
                AND f.youtube_id=a.youtube_id)
      AND (?3 IS NULL OR v.rowid IN (SELECT rowid FROM videos_fts WHERE videos_fts MATCH ?3))
)";

/// The collapsed tree: channels (and the directly-allowed group) with counts and the policies
/// under each. Videos are fetched per group by [`library_videos`].
pub fn library_groups(conn: &Connection, f: &LibraryFilter) -> Result<Vec<LibraryGroup>> {
    let fts = f.q.as_deref().and_then(crate::browse::fts_query);
    let args = params![f.review as i64, f.child, fts];
    let mut s = conn.prepare(&format!(
        "{LIBRARY_ENTRIES} SELECT gkey, MAX(channel_title), COUNT(*) FROM e GROUP BY gkey
         ORDER BY gkey='direct' DESC, MAX(channel_title) COLLATE NOCASE, gkey"
    ))?;
    let mut groups: Vec<LibraryGroup> = s
        .query_map(args, |r| {
            Ok(LibraryGroup {
                key: r.get(0)?,
                title: r.get(1)?,
                count: r.get(2)?,
                playlists: vec![],
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    let mut s = conn.prepare(&format!(
        "{LIBRARY_ENTRIES} SELECT gkey, pid, pkind, pyt, COUNT(*) FROM e WHERE pkind!='video'
         GROUP BY gkey, pid ORDER BY pkind, pyt"
    ))?;
    let sources = s.query_map(args, |r| {
        Ok((
            r.get::<_, String>(0)?,
            LibrarySource {
                policy_id: r.get(1)?,
                kind: r.get(2)?,
                youtube_id: r.get(3)?,
                count: r.get(4)?,
            },
        ))
    })?;
    for row in sources {
        let (key, src) = row?;
        if let Some(g) = groups.iter_mut().find(|g| g.key == key) {
            g.playlists.push(src);
        }
    }
    Ok(groups)
}

/// One page of a group's videos with each video's play count and watched seconds, summed over
/// `play_grants` (all children, or just `f.child`). `sort` is `title`, `plays` or `time`.
pub fn library_videos(
    conn: &Connection,
    f: &LibraryFilter,
    group: &str,
    policy: Option<&str>,
    sort: &str,
    offset: usize,
    limit: usize,
) -> Result<(i64, Vec<LibraryVideo>)> {
    let fts = f.q.as_deref().and_then(crate::browse::fts_query);
    let order = match sort {
        "plays" => "plays DESC, e.title COLLATE NOCASE, e.youtube_id",
        "time" => "watched DESC, e.title COLLATE NOCASE, e.youtube_id",
        "title" | "" => "e.title COLLATE NOCASE, e.youtube_id",
        other => return Err(DbError::Invalid(format!("unknown sort {other:?}"))),
    };
    let mut s = conn.prepare(&format!(
        "{LIBRARY_ENTRIES}
         SELECT e.youtube_id, e.title, e.channel_title, e.pid, e.pkind, e.pyt, e.child_id,
                COALESCE(g.n,0) AS plays, COALESCE(g.t,0) AS watched, COUNT(*) OVER (), e.duration_s
         FROM e LEFT JOIN (SELECT youtube_id, COUNT(*) AS n, SUM(COALESCE(watched_s,0)) AS t
                           FROM play_grants WHERE ?2 IS NULL OR child_id=?2 GROUP BY youtube_id) g
              ON g.youtube_id=e.youtube_id
         WHERE e.gkey=?4 AND (?5 IS NULL OR e.pid=?5)
         ORDER BY {order} LIMIT ?6 OFFSET ?7"
    ))?;
    let mut total = 0;
    let rows = s.query_map(
        params![
            f.review as i64,
            f.child,
            fts,
            group,
            policy,
            limit as i64,
            offset as i64
        ],
        |r| {
            total = r.get(9)?;
            Ok(LibraryVideo {
                youtube_id: r.get(0)?,
                title: r.get(1)?,
                channel_title: r.get(2)?,
                policy_id: r.get(3)?,
                policy_kind: r.get(4)?,
                policy_youtube_id: r.get(5)?,
                child_id: r.get(6)?,
                plays: r.get(7)?,
                watched_s: r.get(8)?,
                duration_s: r.get(10)?,
            })
        },
    )?;
    let videos: Vec<LibraryVideo> = rows.collect::<rusqlite::Result<_>>()?;
    Ok((total, videos))
}

pub fn open_requests(conn: &Connection) -> Result<Vec<OpenRequest>> {
    let mut s = conn.prepare(
        "SELECT q.id, q.child_id, c.name, q.kind, q.youtube_id, COALESCE(v.title,''),
                v.channel_id, COALESCE(v.channel_title,''), q.created_at
         FROM requests q JOIN children c ON c.id=q.child_id
         LEFT JOIN videos v ON v.youtube_id=q.youtube_id
         WHERE q.resolved_at IS NULL ORDER BY q.created_at, q.id",
    )?;
    let rows = s.query_map([], |r| {
        Ok(OpenRequest {
            id: r.get(0)?,
            child_id: r.get(1)?,
            child_name: r.get(2)?,
            kind: r.get(3)?,
            youtube_id: r.get(4)?,
            title: r.get(5)?,
            channel_id: r.get(6)?,
            channel_title: r.get(7)?,
            created_at: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// A child's ask: `like` or `more-from-channel` about a video they may play. A second open
/// request of the same kind for the same video from the same child returns the existing id.
/// `None` means the video is not playable for that child.
pub fn add_request(
    conn: &Connection,
    child_id: &str,
    kind: &str,
    youtube_id: &str,
    now: i64,
) -> Result<Option<String>> {
    if !matches!(kind, "like" | "more-from-channel") {
        return Err(DbError::Invalid(format!("unknown request kind {kind:?}")));
    }
    let playable: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM playable WHERE child_id=?1 AND youtube_id=?2)",
        params![child_id, youtube_id],
        |r| r.get(0),
    )?;
    if !playable {
        return Ok(None);
    }
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM requests WHERE child_id=?1 AND kind=?2 AND youtube_id=?3
             AND resolved_at IS NULL",
            params![child_id, kind, youtube_id],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = existing {
        return Ok(Some(id));
    }
    let id = ulid::Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO requests(id,child_id,kind,youtube_id,created_at) VALUES (?1,?2,?3,?4,?5)",
        params![id, child_id, kind, youtube_id, now],
    )?;
    Ok(Some(id))
}

/// Resolves an open request: `allow-channel` adds a channel policy for the requesting child,
/// `allow-more` approves the `n` newest not-yet-allowed known videos of the video's channel for
/// that child, `dismiss` just closes it. Returns the request's kind, or `None` if it is not open.
pub fn resolve_request(
    conn: &Connection,
    id: &str,
    action: &str,
    n: usize,
    now: i64,
) -> Result<Option<String>> {
    let resolution = match action {
        "allow-channel" => "allowed-channel",
        "allow-more" => "allowed-more",
        "dismiss" => "dismissed",
        other => return Err(DbError::Invalid(format!("unknown action {other:?}"))),
    };
    let row: Option<(String, String, String, Option<String>)> = conn
        .query_row(
            "SELECT q.child_id, q.kind, q.youtube_id, v.channel_id FROM requests q
             LEFT JOIN videos v ON v.youtube_id=q.youtube_id
             WHERE q.id=?1 AND q.resolved_at IS NULL",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()?;
    let Some((child, kind, _video, channel)) = row else {
        return Ok(None);
    };
    let tx = conn.unchecked_transaction()?;
    if action != "dismiss" {
        let channel = channel
            .ok_or_else(|| DbError::Invalid("the video's channel is not known yet".to_string()))?;
        if action == "allow-channel" {
            add_policy(&tx, "channel", &channel, Some(&child), now)?;
        } else {
            let mut s = tx.prepare(
                "SELECT youtube_id FROM videos v WHERE channel_id=?1 AND unavailable_at IS NULL
                   AND NOT EXISTS (SELECT 1 FROM playable p WHERE p.child_id=?2
                        AND p.youtube_id=v.youtube_id)
                   AND NOT EXISTS (SELECT 1 FROM allowed_videos b WHERE b.youtube_id=v.youtube_id
                        AND b.state='blocked' AND (b.child_id IS NULL OR b.child_id=?2))
                 ORDER BY fetched_at DESC, youtube_id DESC LIMIT ?3",
            )?;
            let ids: Vec<String> = s
                .query_map(params![channel, child, n as i64], |r| r.get(0))?
                .collect::<rusqlite::Result<_>>()?;
            drop(s);
            for vid in ids {
                add_policy(&tx, "video", &vid, Some(&child), now)?;
            }
        }
    }
    tx.execute(
        "UPDATE requests SET resolved_at=?2, resolution=?3 WHERE id=?1",
        params![id, now, resolution],
    )?;
    tx.commit()?;
    Ok(Some(kind))
}

pub fn get_budget(conn: &Connection, child: &str) -> Result<Budget> {
    Ok(conn
        .query_row(
            "SELECT video_equivalents, minutes_cap, cutoff_mid_play, daily_windows, ve_minutes
             FROM budgets WHERE child_id=?1",
            [child],
            |r| {
                Ok(Budget {
                    video_equivalents: r.get(0)?,
                    minutes_cap: r.get(1)?,
                    cutoff_mid_play: r.get::<_, i64>(2)? != 0,
                    daily_windows: r.get(3)?,
                    ve_minutes: r.get(4)?,
                })
            },
        )
        .optional()?
        .unwrap_or_default())
}

pub fn put_budget(conn: &Connection, child: &str, b: &Budget) -> Result<()> {
    conn.execute(
        "INSERT INTO budgets(child_id, video_equivalents, minutes_cap, cutoff_mid_play, daily_windows, ve_minutes)
         VALUES (?1,?2,?3,?4,?5,?6)
         ON CONFLICT(child_id) DO UPDATE SET video_equivalents=?2, minutes_cap=?3,
            cutoff_mid_play=?4, daily_windows=?5, ve_minutes=?6",
        params![
            child,
            b.video_equivalents,
            b.minutes_cap,
            b.cutoff_mid_play as i64,
            b.daily_windows,
            b.ve_minutes
        ],
    )?;
    Ok(())
}

/// A child's grants, newest first.
pub fn history(conn: &Connection, child: &str, limit: usize) -> Result<Vec<Grant>> {
    let mut s = conn.prepare(
        "SELECT g.id, g.youtube_id, COALESCE(v.title,''), g.granted_at, COALESCE(g.watched_s,0),
                COALESCE(g.charge,0), g.ended_at, g.end_reason,
                CASE WHEN v.thumb_path IS NOT NULL THEN '/thumbs/' || v.thumb_path || '?v=' || COALESCE(v.fetched_at,0) END
         FROM play_grants g LEFT JOIN videos v ON v.youtube_id=g.youtube_id
         WHERE g.child_id=?1 ORDER BY g.granted_at DESC, g.id DESC LIMIT ?2",
    )?;
    let rows = s.query_map(params![child, limit as i64], |r| {
        Ok(Grant {
            id: r.get(0)?,
            youtube_id: r.get(1)?,
            title: r.get(2)?,
            granted_at: r.get(3)?,
            watched_s: r.get(4)?,
            charge: r.get(5)?,
            ended_at: r.get(6)?,
            end_reason: r.get(7)?,
            thumb: r.get(8)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn list_devices(conn: &Connection) -> Result<Vec<DeviceRow>> {
    let mut s = conn.prepare(
        "SELECT id, child_id, label, paired_at, last_seen, revoked_at FROM devices
         ORDER BY paired_at, id",
    )?;
    let rows = s.query_map([], |r| {
        Ok(DeviceRow {
            id: r.get(0)?,
            child_id: r.get(1)?,
            label: r.get(2)?,
            paired_at: r.get(3)?,
            last_seen: r.get(4)?,
            revoked_at: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn create_child(conn: &Connection, name: &str, avatar: &str, tz: &str) -> Result<String> {
    let id = ulid::Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO children(id,name,avatar,tz) VALUES (?1,?2,?3,?4)",
        params![id, name, avatar, tz],
    )?;
    Ok(id)
}

/// False if there is no such child.
pub fn update_child(
    conn: &Connection,
    id: &str,
    name: &str,
    avatar: &str,
    tz: &str,
) -> Result<bool> {
    Ok(conn.execute(
        "UPDATE children SET name=?2, avatar=?3, tz=?4 WHERE id=?1",
        params![id, name, avatar, tz],
    )? > 0)
}

pub fn list_children(conn: &Connection) -> Result<Vec<ChildRow>> {
    let mut s = conn.prepare("SELECT id, name, avatar, tz FROM children ORDER BY name, id")?;
    let rows = s.query_map([], |r| {
        Ok(ChildRow {
            id: r.get(0)?,
            name: r.get(1)?,
            avatar: r.get(2)?,
            tz: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::migrate_family;

    fn mem() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        migrate_family(&mut c).unwrap();
        c.execute_batch(
            "INSERT INTO children(id,name,avatar,tz) VALUES ('a','A','','UTC'),('b','B','','UTC');
             INSERT INTO videos(youtube_id,title,channel_id,channel_title,fetched_at) VALUES
               ('v1','one','ch1','Chan',1),('v2','two','ch1','Chan',2),('v3','three','ch1','Chan',3),
               ('w1','other','ch2','Other',1);",
        )
        .unwrap();
        c
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

    #[test]
    fn allowing_a_channel_makes_its_known_videos_playable_and_auto_allowed() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        assert_eq!(playable(&c, "a"), ["v1", "v2", "v3"]);
        assert_eq!(auto_allowed(&c, None).unwrap().len(), 3);
    }

    #[test]
    fn allowing_the_same_thing_twice_makes_one_policy() {
        let c = mem();
        let p = add_policy(&c, "channel", "ch1", None, 5).unwrap();
        assert_eq!(add_policy(&c, "channel", "ch1", None, 6).unwrap(), p);
        assert_eq!(list_policies(&c).unwrap().len(), 1);
    }

    #[test]
    fn blocking_one_video_leaves_the_channel_allowed() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        block_video(&c, "v2", None, 6).unwrap();
        assert_eq!(playable(&c, "a"), ["v1", "v3"]);
        assert_eq!(list_policies(&c).unwrap().len(), 1);
        assert!(
            auto_allowed(&c, None)
                .unwrap()
                .iter()
                .all(|i| i.youtube_id != "v2")
        );
    }

    #[test]
    fn a_block_for_one_child_leaves_the_other() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        block_video(&c, "v2", Some("a"), 6).unwrap();
        assert_eq!(playable(&c, "a"), ["v1", "v3"]);
        assert_eq!(playable(&c, "b"), ["v1", "v2", "v3"]);
    }

    #[test]
    fn unblocking_restores_the_video() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        block_video(&c, "v2", None, 6).unwrap();
        unblock_video(&c, "v2", None).unwrap();
        assert_eq!(playable(&c, "a"), ["v1", "v2", "v3"]);
    }

    #[test]
    fn approving_takes_a_video_out_of_the_review_list_and_keeps_it_playable() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        approve_video(&c, "v1", 6).unwrap();
        let ids: Vec<_> = auto_allowed(&c, None)
            .unwrap()
            .into_iter()
            .map(|i| i.youtube_id)
            .collect();
        assert_eq!(ids, ["v2", "v3"]);
        assert_eq!(playable(&c, "a"), ["v1", "v2", "v3"]);
    }

    #[test]
    fn the_review_list_filters_by_channel() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        add_policy(&c, "channel", "ch2", None, 5).unwrap();
        let items = auto_allowed(&c, Some("ch2")).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].youtube_id, "w1");
        assert_eq!(items[0].policy_youtube_id, "ch2");
    }

    #[test]
    fn a_video_policy_is_approved_not_auto_allowed() {
        let c = mem();
        add_policy(&c, "video", "v1", Some("a"), 5).unwrap();
        assert_eq!(playable(&c, "a"), ["v1"]);
        assert!(playable(&c, "b").is_empty());
        assert!(auto_allowed(&c, None).unwrap().is_empty());
    }

    #[test]
    fn removing_a_policy_stops_the_videos_it_allowed() {
        let c = mem();
        let p = add_policy(&c, "channel", "ch1", None, 5).unwrap();
        assert!(remove_policy(&c, &p, 6).unwrap());
        assert!(playable(&c, "a").is_empty());
        assert!(list_policies(&c).unwrap().is_empty());
        assert!(!remove_policy(&c, &p, 7).unwrap());
    }

    #[test]
    fn a_bad_kind_is_refused() {
        let c = mem();
        assert!(add_policy(&c, "show", "x", None, 5).is_err());
    }

    fn request(c: &Connection, id: &str, kind: &str, vid: &str) {
        c.execute(
            "INSERT INTO requests(id,child_id,kind,youtube_id,created_at) VALUES (?1,'a',?2,?3,1)",
            rusqlite::params![id, kind, vid],
        )
        .unwrap();
    }

    #[test]
    fn allowing_the_channel_of_a_request_adds_a_policy_for_that_child() {
        let c = mem();
        request(&c, "r1", "more-from-channel", "v1");
        assert_eq!(open_requests(&c).unwrap().len(), 1);
        let kind = resolve_request(&c, "r1", "allow-channel", 0, 9)
            .unwrap()
            .unwrap();
        assert_eq!(kind, "more-from-channel");
        let ps = list_policies(&c).unwrap();
        assert_eq!(
            (ps[0].kind.as_str(), ps[0].youtube_id.as_str()),
            ("channel", "ch1")
        );
        assert_eq!(ps[0].child_id.as_deref(), Some("a"));
        assert_eq!(playable(&c, "a"), ["v1", "v2", "v3"]);
        assert!(playable(&c, "b").is_empty());
        assert!(open_requests(&c).unwrap().is_empty());
    }

    #[test]
    fn allow_n_more_approves_that_many_of_the_channels_newest_unallowed_videos() {
        let c = mem();
        add_policy(&c, "video", "v1", Some("a"), 1).unwrap();
        request(&c, "r1", "more-from-channel", "v1");
        resolve_request(&c, "r1", "allow-more", 1, 9)
            .unwrap()
            .unwrap();
        assert_eq!(playable(&c, "a"), ["v1", "v3"]);
    }

    #[test]
    fn dismissing_resolves_without_a_policy() {
        let c = mem();
        request(&c, "r1", "like", "v1");
        resolve_request(&c, "r1", "dismiss", 0, 9).unwrap().unwrap();
        assert!(list_policies(&c).unwrap().is_empty());
        let res: String = c
            .query_row("SELECT resolution FROM requests WHERE id='r1'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(res, "dismissed");
    }

    #[test]
    fn a_resolved_or_unknown_request_resolves_to_nothing() {
        let c = mem();
        request(&c, "r1", "like", "v1");
        resolve_request(&c, "r1", "dismiss", 0, 9).unwrap();
        assert_eq!(resolve_request(&c, "r1", "dismiss", 0, 9).unwrap(), None);
        assert_eq!(resolve_request(&c, "nope", "dismiss", 0, 9).unwrap(), None);
        assert!(resolve_request(&c, "r1", "frobnicate", 0, 9).is_err());
    }

    #[test]
    fn budgets_round_trip_and_a_missing_row_is_unlimited() {
        let c = mem();
        assert_eq!(get_budget(&c, "a").unwrap(), Budget::default());
        let b = Budget {
            video_equivalents: Some(3.0),
            minutes_cap: Some(60),
            ve_minutes: Some(10.0),
            cutoff_mid_play: true,
            daily_windows: Some(r#"[["07:00","19:00"]]"#.into()),
        };
        put_budget(&c, "a", &b).unwrap();
        assert_eq!(get_budget(&c, "a").unwrap(), b);
        put_budget(&c, "a", &Budget::default()).unwrap();
        assert_eq!(get_budget(&c, "a").unwrap(), Budget::default());
    }

    #[test]
    fn history_lists_a_childs_grants_newest_first_with_titles() {
        let c = mem();
        c.execute_batch(
            "INSERT INTO devices(id,child_id,token_hash,paired_at) VALUES ('d','a',x'01',0);
             INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,watched_s,charge,ended_at,end_reason)
               VALUES ('g1','a','d','v1',10,60,0.5,70,'ended'),('g2','a','d','v2',20,5,0,25,'left'),
                      ('g3','b','d','v1',30,1,0,31,'left');",
        )
        .unwrap();
        let h = history(&c, "a", 10).unwrap();
        assert_eq!(
            h.iter().map(|g| g.id.as_str()).collect::<Vec<_>>(),
            ["g2", "g1"]
        );
        assert_eq!(h[1].title, "one");
        assert_eq!(h[1].end_reason.as_deref(), Some("ended"));
    }

    #[test]
    fn devices_list_with_their_revoked_state() {
        let c = mem();
        c.execute_batch(
            "INSERT INTO devices(id,child_id,token_hash,label,paired_at,revoked_at) VALUES
              ('d1','a',x'01','iPad',1,NULL),('d2','a',x'02','old',2,9),('d3','b',x'03','x',3,NULL);",
        )
        .unwrap();
        let d = list_devices(&c).unwrap();
        assert_eq!(d.len(), 3);
        assert_eq!(d.iter().find(|d| d.id == "d2").unwrap().revoked_at, Some(9));
    }

    #[test]
    fn children_can_be_added_updated_and_listed() {
        let c = mem();
        let id = create_child(&c, "Cy", "🦊", "Europe/London").unwrap();
        update_child(&c, &id, "Cyrus", "🐻", "UTC").unwrap();
        let kids = list_children(&c).unwrap();
        let cy = kids.iter().find(|k| k.id == id).unwrap();
        assert_eq!(
            (cy.name.as_str(), cy.avatar.as_str(), cy.tz.as_str()),
            ("Cyrus", "🐻", "UTC")
        );
        assert!(!update_child(&c, "ghost", "x", "", "UTC").unwrap());
    }

    #[test]
    fn the_library_groups_by_channel_counts_plays_and_searches() {
        let c = mem();
        add_policy(&c, "channel", "ch1", None, 5).unwrap();
        add_policy(&c, "video", "w1", None, 5).unwrap();
        c.execute_batch(
            "INSERT INTO devices(id,child_id,token_hash,paired_at) VALUES ('d','a',x'01',0);
             INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,watched_s)
               VALUES ('g1','a','d','v1',10,60),('g2','a','d','v1',20,30),('g3','b','d','v1',30,10);",
        )
        .unwrap();
        let f = LibraryFilter::default();
        let g = library_groups(&c, &f).unwrap();
        assert_eq!(
            g.iter()
                .map(|g| (g.key.as_str(), g.count))
                .collect::<Vec<_>>(),
            [("direct", 1), ("ch1", 3)]
        );
        assert_eq!(g[1].playlists.len(), 1);
        let (total, v) = library_videos(&c, &f, "ch1", None, "plays", 0, 2).unwrap();
        assert_eq!((total, v.len()), (3, 2));
        assert_eq!(
            (v[0].youtube_id.as_str(), v[0].plays, v[0].watched_s),
            ("v1", 3, 100.0)
        );
        let (t, v) = library_videos(&c, &f, "ch1", None, "title", 0, 10).unwrap();
        assert_eq!((t, v.len()), (3, 3));
        let one = LibraryFilter {
            child: Some("a".into()),
            ..f.clone()
        };
        let (_, v) = library_videos(&c, &one, "ch1", None, "time", 0, 10).unwrap();
        assert_eq!((v[0].plays, v[0].watched_s), (2, 90.0));
        let q = LibraryFilter {
            q: Some("thr".into()),
            ..f.clone()
        };
        let g = library_groups(&c, &q).unwrap();
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].count, 1);
        let rev = LibraryFilter { review: true, ..f };
        assert_eq!(library_groups(&c, &rev).unwrap().len(), 1);
        assert!(library_videos(&c, &rev, "ch1", None, "bogus", 0, 1).is_err());
    }

    /// A channel holding a 45 s short, a 25-minute episode and a 90-minute compilation.
    fn rule_channel(c: &Connection) -> String {
        c.execute_batch(
            "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
               ('short','Quick clip','chr','R',45,1),
               ('ep','Episode 4','chr','R',1500,1),
               ('comp','Episode 1-9 compilation','chr','R',5400,1);",
        )
        .unwrap();
        add_policy(c, "channel", "chr", None, 5).unwrap()
    }

    fn no_shorts_under_30() -> Rules {
        Rules {
            exclude_shorts: true,
            max_s: Some(1800),
            ..Rules::default()
        }
    }

    #[test]
    fn no_shorts_and_under_thirty_minutes_allows_only_the_episode() {
        let c = mem();
        let p = rule_channel(&c);
        assert_eq!(playable(&c, "a"), ["comp", "ep", "short"]);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        assert_eq!(playable(&c, "a"), ["ep"]);
    }

    #[test]
    fn a_new_upload_is_judged_by_the_rules_when_it_arrives() {
        let c = mem();
        let p = rule_channel(&c);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        c.execute_batch(
            "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
               ('new-ok','Episode 5','chr','R',1200,9),('new-long','Movie night','chr','R',7200,9),
               ('new-short','Another clip','chr','R',30,9);
             INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES
               ('new-ok',NULL,'%P','auto-allowed',9),('new-long',NULL,'%P','auto-allowed',9),
               ('new-short',NULL,'%P','auto-allowed',9);"
                .replace("%P", &p)
                .as_str(),
        )
        .unwrap();
        assert_eq!(playable(&c, "a"), ["ep", "new-ok"]);
    }

    #[test]
    fn a_video_with_unknown_duration_fails_a_duration_rule() {
        let c = mem();
        let p = add_policy(&c, "channel", "ch1", None, 5).unwrap();
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        assert!(playable(&c, "a").is_empty());
    }

    #[test]
    fn minimum_duration_and_title_words_filter() {
        let c = mem();
        let p = rule_channel(&c);
        let rules = Rules {
            min_s: Some(600),
            title_lacks: vec!["compilation".into()],
            ..Rules::default()
        };
        set_rules(&c, &p, &rules).unwrap();
        assert_eq!(playable(&c, "a"), ["ep"]);
        let rules = Rules {
            title_has: vec!["QUICK".into(), "nothing".into()],
            ..Rules::default()
        };
        set_rules(&c, &p, &rules).unwrap();
        assert_eq!(playable(&c, "a"), ["short"]);
    }

    #[test]
    fn a_quoted_phrase_is_the_same_rule_as_the_bare_phrase() {
        let c = mem();
        c.execute_batch(
            "INSERT INTO videos(youtube_id,title,channel_id,channel_title,duration_s,fetched_at) VALUES
               ('draw','How To Draw a Cat','chr','R',600,1),
               ('other','Cat songs','chr','R',600,1);",
        )
        .unwrap();
        let p = add_policy(&c, "channel", "chr", None, 5).unwrap();
        let rules = |w: &str| Rules {
            title_lacks: vec![w.into()],
            ..Rules::default()
        };
        set_rules(&c, &p, &rules("\"how to\"")).unwrap();
        let quoted = get_rules(&c, &p).unwrap();
        assert_eq!(quoted.title_lacks, ["how to"]);
        assert_eq!(playable(&c, "a"), ["other"]);
        set_rules(&c, &p, &rules("  How To ")).unwrap();
        assert_eq!(get_rules(&c, &p).unwrap(), quoted);
        set_rules(&c, &p, &rules("\u{201c}how to\u{201d}")).unwrap();
        assert_eq!(get_rules(&c, &p).unwrap(), quoted);
        set_rules(&c, &p, &rules("\"\"")).unwrap();
        assert!(get_rules(&c, &p).unwrap().title_lacks.is_empty());
    }

    #[test]
    fn a_phrase_with_a_comma_is_one_phrase_and_removing_it_unfilters() {
        let c = mem();
        let p = rule_channel(&c);
        let rules = Rules {
            title_lacks: vec!["episode 1-9, compilation".into()],
            ..Rules::default()
        };
        set_rules(&c, &p, &rules).unwrap();
        assert_eq!(get_rules(&c, &p).unwrap().title_lacks.len(), 1);
        assert_eq!(playable(&c, "a"), ["comp", "ep", "short"]);
        let rules = Rules {
            title_lacks: vec!["episode 1-9 comp".into()],
            ..Rules::default()
        };
        set_rules(&c, &p, &rules).unwrap();
        assert_eq!(playable(&c, "a"), ["ep", "short"]);
        set_rules(&c, &p, &Rules::default()).unwrap();
        assert_eq!(playable(&c, "a"), ["comp", "ep", "short"]);
    }

    #[test]
    fn rules_bind_only_their_own_policy() {
        let c = mem();
        let p = rule_channel(&c);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        add_policy(&c, "video", "comp", None, 6).unwrap();
        assert_eq!(playable(&c, "a"), ["comp", "ep"]);
    }

    #[test]
    fn an_explicit_allow_beats_the_rules_and_a_block_beats_the_allow() {
        let c = mem();
        let p = rule_channel(&c);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        add_policy(&c, "video", "short", None, 6).unwrap();
        assert_eq!(playable(&c, "a"), ["ep", "short"]);
        block_video(&c, "ep", None, 7).unwrap();
        assert_eq!(playable(&c, "a"), ["short"]);
    }

    #[test]
    fn excluded_videos_leave_the_review_list_and_the_library() {
        let c = mem();
        let p = rule_channel(&c);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        let ids: Vec<_> = auto_allowed(&c, Some("chr"))
            .unwrap()
            .into_iter()
            .map(|i| i.youtube_id)
            .collect();
        assert_eq!(ids, ["ep"]);
        let g = library_groups(&c, &LibraryFilter::default()).unwrap();
        assert_eq!(g.iter().find(|g| g.key == "chr").unwrap().count, 1);
    }

    #[test]
    fn clearing_the_rules_allows_everything_again() {
        let c = mem();
        let p = rule_channel(&c);
        set_rules(&c, &p, &no_shorts_under_30()).unwrap();
        set_rules(&c, &p, &Rules::default()).unwrap();
        assert_eq!(playable(&c, "a"), ["comp", "ep", "short"]);
        assert_eq!(get_rules(&c, &p).unwrap(), Rules::default());
    }

    #[test]
    fn rules_round_trip_and_only_channel_and_playlist_policies_take_them() {
        let c = mem();
        let p = rule_channel(&c);
        let r = Rules {
            exclude_shorts: true,
            min_s: Some(60),
            max_s: Some(1800),
            title_has: vec!["ep".into()],
            title_lacks: vec!["compilation".into(), "mix".into()],
        };
        assert!(set_rules(&c, &p, &r).unwrap());
        assert_eq!(get_rules(&c, &p).unwrap(), r);
        let v = add_policy(&c, "video", "ep", None, 6).unwrap();
        assert!(!set_rules(&c, &v, &r).unwrap());
        assert!(!set_rules(&c, "nope", &r).unwrap());
    }

    #[test]
    fn the_preview_counts_without_saving() {
        let c = mem();
        let p = rule_channel(&c);
        let pv = preview_rules(&c, &p, &no_shorts_under_30(), 10)
            .unwrap()
            .unwrap();
        assert_eq!((pv.allowed, pv.excluded), (1, 2));
        let mut ex: Vec<_> = pv
            .excluded_videos
            .iter()
            .map(|v| v.youtube_id.as_str())
            .collect();
        ex.sort();
        assert_eq!(ex, ["comp", "short"]);
        assert_eq!(get_rules(&c, &p).unwrap(), Rules::default());
        assert_eq!(playable(&c, "a"), ["comp", "ep", "short"]);
        let pv = preview_rules(&c, &p, &no_shorts_under_30(), 1)
            .unwrap()
            .unwrap();
        assert_eq!((pv.excluded, pv.excluded_videos.len()), (2, 1));
    }
}

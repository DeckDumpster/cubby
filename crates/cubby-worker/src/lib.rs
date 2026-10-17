//! The metadata worker: expands allow policies into per-video rows through the YouTube client,
//! refreshes stale metadata, and spends no more than the daily quota.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cubby_otel::MetricsSink;
use cubby_yt::{Client, Meter, Transport};
use rusqlite::Connection;

const DAY_S: i64 = 86_400;

/// What one pass over one family file did.
#[derive(Debug, Default, PartialEq)]
pub struct PassReport {
    /// `ok`, `quota` or `error`.
    pub outcome: &'static str,
    /// Quota units this pass spent, by call.
    pub units: BTreeMap<String, u32>,
}

/// The day's quota counter, kept in the family file so it survives between timer runs.
pub struct DbMeter<'a> {
    conn: &'a Connection,
    day: String,
    run: BTreeMap<String, u32>,
}

impl<'a> DbMeter<'a> {
    pub fn new(conn: &'a Connection, now: i64) -> Self {
        DbMeter {
            conn,
            day: (now.div_euclid(DAY_S)).to_string(),
            run: BTreeMap::new(),
        }
    }
}

impl Meter for DbMeter<'_> {
    fn used_today(&self) -> u32 {
        self.conn
            .query_row(
                "SELECT COALESCE(SUM(units), 0) FROM yt_quota WHERE day = ?",
                [&self.day],
                |r| r.get(0),
            )
            .unwrap_or(u32::MAX)
    }
    fn record(&mut self, call: &str, units: u32) {
        let _ = self.conn.execute(
            "INSERT INTO yt_quota(day, call, units) VALUES(?, ?, ?)
             ON CONFLICT(day, call) DO UPDATE SET units = units + excluded.units",
            rusqlite::params![self.day, call, units],
        );
        *self.run.entry(call.to_string()).or_default() += units;
    }
}

struct Policy {
    id: String,
    kind: String,
    youtube_id: String,
    child_id: Option<String>,
}

/// One pass over `conn`: video policies, then channel and playlist expansion, then a refresh of
/// stale metadata. Stops at the first quota refusal.
pub fn run_pass<T: Transport>(
    conn: &Connection,
    transport: T,
    now: i64,
    thumbs: Option<&Path>,
) -> PassReport {
    let mut client = Client::new(transport, DbMeter::new(conn, now));
    let mut outcome = "ok";
    let r = pass(conn, &mut client, now, thumbs, &mut outcome);
    if let Err(YtErr::Quota) = r {
        outcome = "quota";
    }
    PassReport {
        outcome,
        units: client.meter().run.clone(),
    }
}

enum YtErr {
    Quota,
}

/// Records a failed call: quota stops the pass, anything else marks it `error` and moves on.
fn soft<V>(
    r: Result<V, cubby_yt::YtError>,
    outcome: &mut &'static str,
) -> Result<Option<V>, YtErr> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(cubby_yt::YtError::QuotaExhausted) => Err(YtErr::Quota),
        Err(e) => {
            eprintln!("cubby-worker: {e}");
            *outcome = "error";
            Ok(None)
        }
    }
}

fn pass<T: Transport>(
    conn: &Connection,
    client: &mut Client<T, DbMeter<'_>>,
    now: i64,
    thumbs: Option<&Path>,
    outcome: &mut &'static str,
) -> Result<(), YtErr> {
    let policies: Vec<Policy> = conn
        .prepare(
            "SELECT id, kind, youtube_id, child_id FROM allow_policies
             WHERE removed_at IS NULL ORDER BY added_at, id",
        )
        .and_then(|mut q| {
            q.query_map([], |r| {
                Ok(Policy {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    youtube_id: r.get(2)?,
                    child_id: r.get(3)?,
                })
            })?
            .collect()
        })
        .unwrap_or_else(|e| {
            eprintln!("cubby-worker: reading policies: {e}");
            *outcome = "error";
            Vec::new()
        });

    let picked: Vec<&Policy> = policies.iter().filter(|p| p.kind == "video").collect();
    let mut ids: Vec<String> = picked.iter().map(|p| p.youtube_id.clone()).collect();
    ids.dedup();
    if soft(ingest(conn, client, &ids, now, thumbs), outcome)?.is_some() {
        for p in picked {
            allow(conn, &p.youtube_id, p, "approved", now);
        }
    }
    for p in policies.iter().filter(|p| p.kind != "video") {
        expand(conn, client, p, now, thumbs, outcome)?;
    }
    refresh(conn, client, now, thumbs, outcome)
}

fn allow(conn: &Connection, id: &str, p: &Policy, state: &str, now: i64) {
    let _ = conn.execute(
        "INSERT INTO allowed_videos(youtube_id, child_id, policy_id, state, changed_at)
         SELECT ?1, ?2, ?3, ?4, ?5
         WHERE NOT EXISTS (SELECT 1 FROM allowed_videos
                           WHERE youtube_id = ?1 AND child_id IS ?2 AND policy_id = ?3)",
        rusqlite::params![id, p.child_id, p.id, state, now],
    );
}

/// Pages a channel's uploads (or a playlist) until a known id, then fetches metadata for the new
/// ids and writes their rows. Nothing is written unless the whole expansion completed.
fn expand<T: Transport>(
    conn: &Connection,
    client: &mut Client<T, DbMeter<'_>>,
    p: &Policy,
    now: i64,
    thumbs: Option<&Path>,
    outcome: &mut &'static str,
) -> Result<(), YtErr> {
    let playlist = if p.kind == "playlist" {
        p.youtube_id.clone()
    } else {
        let cached: Option<String> = conn
            .query_row(
                "SELECT uploads_playlist FROM policy_cursor WHERE policy_id = ?",
                [&p.id],
                |r| r.get(0),
            )
            .ok();
        match cached {
            Some(pl) => pl,
            None => {
                let Some(Some(pl)) = soft(client.uploads_playlist(&p.youtube_id), outcome)? else {
                    return Ok(());
                };
                let _ = conn.execute(
                    "INSERT OR REPLACE INTO policy_cursor VALUES(?, ?)",
                    [&p.id, &pl],
                );
                pl
            }
        }
    };
    let known = |id: &str| -> bool {
        conn.query_row(
            "SELECT 1 FROM allowed_videos WHERE youtube_id = ? AND child_id IS ? AND policy_id = ?",
            rusqlite::params![id, p.child_id, p.id],
            |_| Ok(()),
        )
        .is_ok()
    };
    let mut new_ids: Vec<String> = Vec::new();
    let mut token: Option<String> = None;
    'pages: loop {
        let Some(page) = soft(client.playlist_items(&playlist, token.as_deref()), outcome)? else {
            return Ok(());
        };
        for id in page.ids {
            if known(&id) {
                break 'pages;
            }
            if !new_ids.contains(&id) {
                new_ids.push(id);
            }
        }
        match page.next_page_token {
            Some(t) => token = Some(t),
            None => break,
        }
    }
    let fresh = |id: &str| -> bool {
        conn.query_row(
            "SELECT 1 FROM videos WHERE youtube_id = ? AND fetched_at > ?",
            rusqlite::params![id, now - DAY_S],
            |_| Ok(()),
        )
        .is_ok()
    };
    let need: Vec<String> = new_ids.iter().filter(|i| !fresh(i)).cloned().collect();
    if soft(ingest(conn, client, &need, now, thumbs), outcome)?.is_none() {
        return Ok(());
    }
    for id in &new_ids {
        allow(conn, id, p, "auto-allowed", now);
    }
    Ok(())
}

/// Fetches `ids` in batches and upserts their `videos` rows; ids that come back private or
/// missing get `unavailable_at` set.
fn ingest<T: Transport>(
    conn: &Connection,
    client: &mut Client<T, DbMeter<'_>>,
    ids: &[String],
    now: i64,
    thumbs: Option<&Path>,
) -> Result<(), cubby_yt::YtError> {
    for batch in ids.chunks(cubby_yt::BATCH) {
        let r = client.videos_list(batch)?;
        for v in &r.available {
            let thumb = thumbs.and_then(|dir| fetch_thumb(conn, client, v, dir));
            let _ = conn.execute(
                "INSERT INTO videos(youtube_id, title, channel_id, channel_title, tags,
                                    duration_s, thumb_path, made_for_kids, fetched_at, unavailable_at)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, NULL)
                 ON CONFLICT(youtube_id) DO UPDATE SET
                    title = ?2, channel_id = ?3, channel_title = ?4, tags = ?5, duration_s = ?6,
                    thumb_path = COALESCE(?7, thumb_path), made_for_kids = ?8, fetched_at = ?9,
                    unavailable_at = NULL",
                rusqlite::params![
                    v.id,
                    v.title,
                    v.channel_id,
                    v.channel_title,
                    serde_tags(&v.tags),
                    v.duration_s as i64,
                    thumb,
                    v.made_for_kids,
                    now
                ],
            );
        }
        for id in &r.unavailable {
            let _ = conn.execute(
                "INSERT INTO videos(youtube_id, title, fetched_at, unavailable_at)
                 VALUES(?1, '', ?2, ?2)
                 ON CONFLICT(youtube_id) DO UPDATE SET
                    fetched_at = ?2, unavailable_at = COALESCE(unavailable_at, ?2)",
                rusqlite::params![id, now],
            );
        }
    }
    Ok(())
}

fn serde_tags(tags: &[String]) -> String {
    format!(
        "[{}]",
        tags.iter()
            .map(|t| format!("{:?}", t))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Downloads the thumbnail once; returns the path relative to the thumbs dir, or `None` if there
/// is already one, no thumbnail, or the download failed.
fn fetch_thumb<T: Transport>(
    conn: &Connection,
    client: &Client<T, DbMeter<'_>>,
    v: &cubby_yt::Video,
    dir: &Path,
) -> Option<String> {
    let url = v.thumb_url.as_deref()?;
    let have: Option<String> = conn
        .query_row(
            "SELECT thumb_path FROM videos WHERE youtube_id = ?",
            [&v.id],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    if have.is_some_and(|p| dir.join(p).exists()) {
        return None;
    }
    let bytes = client.thumbnail(url).ok()?;
    std::fs::create_dir_all(dir).ok()?;
    let name = format!("{}.jpg", v.id);
    std::fs::write(dir.join(&name), bytes).ok()?;
    Some(name)
}

/// Re-fetches every video whose metadata is over a day old, so a video that went private drops
/// out and one that came back returns.
fn refresh<T: Transport>(
    conn: &Connection,
    client: &mut Client<T, DbMeter<'_>>,
    now: i64,
    thumbs: Option<&Path>,
    outcome: &mut &'static str,
) -> Result<(), YtErr> {
    let stale: Vec<String> = conn
        .prepare("SELECT youtube_id FROM videos WHERE fetched_at < ? ORDER BY fetched_at")
        .and_then(|mut q| q.query_map([now - DAY_S], |r| r.get(0))?.collect())
        .unwrap_or_default();
    soft(ingest(conn, client, &stale, now, thumbs), outcome)?;
    Ok(())
}

pub struct Config {
    pub data_dir: PathBuf,
    /// Empty disables every network call.
    pub api_key: String,
    pub otlp_endpoint: String,
}

impl Config {
    pub fn from_env() -> Config {
        let var = |k: &str| std::env::var(k).unwrap_or_default();
        Config {
            data_dir: PathBuf::from(std::env::var("CUBBY_DATA_DIR").unwrap_or("/data".into())),
            api_key: var("CUBBY_YT_API_KEY"),
            otlp_endpoint: var("CUBBY_OTLP_ENDPOINT"),
        }
    }
}

/// Runs one pass over every family file in the data dir and pushes the run's metrics.
pub fn run_once(config: &Config) -> &'static str {
    let sink = MetricsSink::new(Some(&config.otlp_endpoint), "cubby-worker");
    let start = Instant::now();
    let mut outcome = "ok";
    let mut units: BTreeMap<String, u32> = BTreeMap::new();
    if config.api_key.is_empty() {
        outcome = "disabled";
    } else {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs() as i64);
        let transport = cubby_yt::HttpTransport::new(&config.api_key);
        for path in family_files(&config.data_dir) {
            let conn = match cubby_db::open_family(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("cubby-worker: {}: {e}", path.display());
                    outcome = "error";
                    continue;
                }
            };
            let r = run_pass(
                &conn,
                &transport,
                now,
                Some(&config.data_dir.join("thumbs")),
            );
            for (call, n) in r.units {
                *units.entry(call).or_default() += n;
            }
            outcome = worst(outcome, r.outcome);
        }
    }
    for (call, n) in &units {
        sink.push_value(
            "cubby_yt_quota_units",
            f64::from(*n),
            "1",
            &[("call", call)],
        );
    }
    sink.push_seconds(
        "cubby_worker_run_seconds",
        start.elapsed().as_secs_f64(),
        &[("outcome", outcome)],
    );
    outcome
}

fn family_files(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("family-") && n.ends_with(".sqlite"))
        })
        .collect();
    v.sort();
    v
}

fn worst(a: &'static str, b: &'static str) -> &'static str {
    let rank = |o| {
        ["ok", "quota", "error"]
            .iter()
            .position(|x| *x == o)
            .unwrap_or(0)
    };
    if rank(b) > rank(a) { b } else { a }
}

#[cfg(test)]
mod tests;

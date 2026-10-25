//! The child play protocol: `play` grants a watch, `beat` measures it, `end` closes it. The
//! budget decisions live in `cubby_core::budget`; this module reads and writes the grants.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::routing::post;
use axum::{Extension, Json, Router};
use cubby_core::budget::{self, Budget, Progress, Refusal, Remaining, Usage};
use cubby_db::identity::Device;
use rusqlite::{Connection, OptionalExtension, params};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::AppState;
use crate::error::ApiError;
use crate::identity::{ParentOf, open_own};

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[derive(Debug, PartialEq)]
pub enum PlayError {
    /// The grant is not this child's, or does not exist.
    NoGrant,
    /// The grant is already closed.
    Closed,
    BadInput(&'static str),
    Db(String),
}

impl From<rusqlite::Error> for PlayError {
    fn from(e: rusqlite::Error) -> Self {
        PlayError::Db(e.to_string())
    }
}

#[derive(Debug, PartialEq)]
pub enum PlayOutcome {
    Granted {
        grant_id: String,
        remaining: Remaining,
    },
    Refused {
        reason: &'static str,
        next_start: Option<i64>,
    },
}

#[derive(Debug, PartialEq)]
pub struct BeatOutcome {
    pub remaining: Remaining,
    /// `Some(grace_s)` when the page must stop (only with `cutoff_mid_play` on).
    pub stop: Option<u32>,
    pub watched_delta_s: f64,
    pub charge_delta: f64,
}

fn child_tz(conn: &Connection, child: &str) -> rusqlite::Result<String> {
    conn.query_row("SELECT tz FROM children WHERE id=?1", [child], |r| r.get(0))
}

/// No budget row means no limits.
fn load_budget(conn: &Connection, child: &str) -> rusqlite::Result<Budget> {
    let row = conn
        .query_row(
            "SELECT video_equivalents, minutes_cap, cutoff_mid_play, daily_windows, ve_minutes
             FROM budgets WHERE child_id=?1",
            [child],
            |r| {
                Ok((
                    r.get::<_, Option<f64>>(0)?,
                    r.get::<_, Option<f64>>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, Option<f64>>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((ve, minutes_cap, cutoff, windows, ve_minutes)) = row else {
        return Ok(Budget {
            video_equivalents: None,
            minutes_cap: None,
            ve_minutes: budget::DEFAULT_VE_MINUTES,
            cutoff_mid_play: false,
            daily_windows: vec![],
        });
    };
    // A windows column we cannot parse closes the day rather than opening it.
    let daily_windows = match windows.as_deref().map(str::trim).filter(|w| !w.is_empty()) {
        None => vec![],
        Some(w) => serde_json::from_str::<Vec<(String, String)>>(w)
            .ok()
            .and_then(|p| budget::parse_windows(&p))
            .unwrap_or_else(|| vec![(0, 0)]),
    };
    Ok(Budget {
        video_equivalents: ve,
        minutes_cap,
        ve_minutes: ve_minutes
            .filter(|m| m.is_finite() && *m > 0.0)
            .unwrap_or(budget::DEFAULT_VE_MINUTES),
        cutoff_mid_play: cutoff != 0,
        daily_windows,
    })
}

/// Totals since the child's last local midnight.
fn usage(conn: &Connection, child: &str, tz: &str, now: i64) -> rusqlite::Result<Usage> {
    conn.query_row(
        "SELECT COALESCE(SUM(charge),0), COALESCE(SUM(watched_s),0) FROM play_grants
         WHERE child_id=?1 AND granted_at>=?2",
        params![child, budget::day_start(tz, now)],
        |r| {
            Ok(Usage {
                charge: r.get(0)?,
                watched_s: r.get(1)?,
            })
        },
    )
}

/// Closes every grant that has gone [`budget::BEAT_TIMEOUT_S`] without a beat. The charge was
/// kept current by the beats, so closing only stamps the end.
pub fn reap_stale(conn: &Connection, now: i64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE play_grants SET ended_at=COALESCE(last_beat_at,granted_at), end_reason='timeout'
         WHERE ended_at IS NULL AND COALESCE(last_beat_at,granted_at) < ?1",
        [now - budget::BEAT_TIMEOUT_S],
    )
}

pub fn play(
    conn: &Connection,
    dev: &Device,
    youtube_id: &str,
    now: i64,
) -> Result<PlayOutcome, PlayError> {
    reap_stale(conn, now)?;
    let duration: Option<f64> = conn
        .query_row(
            "SELECT v.duration_s FROM playable p JOIN videos v USING(youtube_id)
             WHERE p.child_id=?1 AND p.youtube_id=?2",
            params![dev.child_id, youtube_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    // Without a duration a play cannot be charged, so it cannot be granted.
    if duration.is_none_or(|d| d <= 0.0) {
        return Ok(PlayOutcome::Refused {
            reason: "not-allowed",
            next_start: None,
        });
    }
    let b = load_budget(conn, &dev.child_id)?;
    let tz = child_tz(conn, &dev.child_id)?;
    let u = usage(conn, &dev.child_id, &tz, now)?;
    if let Err(r) = budget::may_start(&b, u, &tz, now) {
        return Ok(PlayOutcome::Refused {
            reason: r.reason(),
            next_start: match r {
                Refusal::NotNow { next_start } => Some(next_start),
                _ => None,
            },
        });
    }
    // One watch at a time per device: starting another leaves the first.
    conn.execute(
        "UPDATE play_grants SET ended_at=?2, end_reason='left' WHERE device_id=?1 AND ended_at IS NULL",
        params![dev.id, now],
    )?;
    let grant_id = ulid::Ulid::generate().to_string();
    conn.execute(
        "INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,last_beat_at,max_position_s,watched_s,charge,ve_s)
         VALUES (?1,?2,?3,?4,?5,?5,0,0,0,?6)",
        params![grant_id, dev.child_id, dev.id, youtube_id, now, b.ve_minutes * 60.0],
    )?;
    Ok(PlayOutcome::Granted {
        grant_id,
        remaining: budget::remaining(&b, u),
    })
}

struct Grant {
    youtube_id: String,
    progress: Progress,
    charge: f64,
    ended: bool,
    /// Seconds per video equivalent the grant was stamped with; `None` for a grant that
    /// predates time-based charging.
    ve_s: Option<f64>,
}

fn load_grant(conn: &Connection, dev: &Device, id: &str) -> Result<Grant, PlayError> {
    conn.query_row(
        "SELECT youtube_id, COALESCE(last_beat_at,granted_at), COALESCE(max_position_s,0),
                COALESCE(watched_s,0), COALESCE(charge,0), ended_at IS NOT NULL, ve_s
         FROM play_grants WHERE id=?1 AND child_id=?2",
        params![id, dev.child_id],
        |r| {
            Ok(Grant {
                youtube_id: r.get(0)?,
                progress: Progress {
                    last_beat_at: r.get(1)?,
                    position_s: r.get(2)?,
                    watched_s: r.get(3)?,
                },
                charge: r.get(4)?,
                ended: r.get(5)?,
                ve_s: r.get(6)?,
            })
        },
    )
    .optional()?
    .ok_or(PlayError::NoGrant)
}

/// Folds a heartbeat into the grant (`max_position_s` holds the position at the last beat) and
/// answers with what is left. The client's `position_s` is a position, never watched time.
pub fn beat(
    conn: &Connection,
    dev: &Device,
    grant_id: &str,
    position_s: f64,
    now: i64,
) -> Result<BeatOutcome, PlayError> {
    if !position_s.is_finite() || position_s < 0.0 {
        return Err(PlayError::BadInput("position_s"));
    }
    reap_stale(conn, now)?;
    let g = load_grant(conn, dev, grant_id)?;
    if g.ended {
        return Err(PlayError::Closed);
    }
    let duration: f64 = conn
        .query_row(
            "SELECT COALESCE(duration_s,0) FROM videos WHERE youtube_id=?1",
            [&g.youtube_id],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0.0);
    let p = budget::apply_beat(g.progress, now, position_s);
    let new_charge = budget::charge(p.watched_s, duration, g.ve_s);
    conn.execute(
        "UPDATE play_grants SET last_beat_at=?2, max_position_s=?3, watched_s=?4, charge=?5 WHERE id=?1",
        params![grant_id, now, p.position_s, p.watched_s, new_charge],
    )?;
    let b = load_budget(conn, &dev.child_id)?;
    let tz = child_tz(conn, &dev.child_id)?;
    let u = usage(conn, &dev.child_id, &tz, now)?;
    Ok(BeatOutcome {
        remaining: budget::remaining(&b, u),
        stop: budget::cutoff(&b, u, &tz, now),
        watched_delta_s: p.watched_s - g.progress.watched_s,
        charge_delta: new_charge - g.charge,
    })
}

pub fn end(
    conn: &Connection,
    dev: &Device,
    grant_id: &str,
    reason: &str,
    now: i64,
) -> Result<(), PlayError> {
    if !matches!(reason, "ended" | "left" | "stop" | "off-list") {
        return Err(PlayError::BadInput("reason"));
    }
    reap_stale(conn, now)?;
    let g = load_grant(conn, dev, grant_id)?;
    if g.ended {
        return Err(PlayError::Closed);
    }
    conn.execute(
        "UPDATE play_grants SET ended_at=?2, end_reason=?3 WHERE id=?1",
        params![grant_id, now, reason],
    )?;
    Ok(())
}

// ---- HTTP ----

/// Mounted behind the device guard, which supplies the family and the [`Device`].
pub(crate) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/api/child/play", post(play_route))
        .route("/api/child/grant/{id}/beat", post(beat_route))
        .route("/api/child/grant/{id}/end", post(end_route))
}

#[derive(Deserialize)]
struct PlayBody {
    youtube_id: String,
}

#[derive(Deserialize)]
struct BeatBody {
    position_s: f64,
    #[serde(default)]
    #[allow(dead_code)]
    state: String,
}

#[derive(Deserialize)]
struct EndBody {
    reason: String,
}

/// What the child has left right now, in the shape `play` and `beat` report it.
pub(crate) fn remaining_json_for(
    conn: &Connection,
    child: &str,
    now: i64,
) -> rusqlite::Result<Value> {
    let b = load_budget(conn, child)?;
    let u = usage(conn, child, &child_tz(conn, child)?, now)?;
    Ok(remaining_json(budget::remaining(&b, u)))
}

/// One child's progress against the limits, from the same budget, usage and clock functions the
/// play path decides with. An unlimited limit is `null`, never a number.
pub(crate) fn progress_json(conn: &Connection, child: &str, now: i64) -> rusqlite::Result<Value> {
    let b = load_budget(conn, child)?;
    let tz = child_tz(conn, child)?;
    let u = usage(conn, child, &tz, now)?;
    let r = budget::remaining(&b, u);
    let live = conn
        .query_row(
            "SELECT g.youtube_id, COALESCE(v.title,''), g.granted_at, COALESCE(g.watched_s,0)
             FROM play_grants g LEFT JOIN videos v ON v.youtube_id=g.youtube_id
             WHERE g.child_id=?1 AND g.ended_at IS NULL AND COALESCE(g.last_beat_at,g.granted_at)>=?2
             ORDER BY g.granted_at DESC LIMIT 1",
            params![child, now - budget::BEAT_TIMEOUT_S],
            |r| {
                Ok(json!({"youtube_id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?,
                    "since": r.get::<_, i64>(2)?, "minutes": r.get::<_, f64>(3)? / 60.0}))
            },
        )
        .optional()?;
    let next_open = budget::next_window_start(&b.daily_windows, &tz, now);
    let closes = budget::current_window_end(&b.daily_windows, &tz, now);
    let side = |limit: Option<f64>, used: f64, left: Option<f64>| {
        json!({"limit": limit, "used": used, "left": left.map(|l| l.max(0.0)),
            "reached": left.is_some_and(|l| l <= 0.0)})
    };
    Ok(json!({
        "child_id": child, "tz": tz,
        "videos": side(b.video_equivalents, u.charge, r.videos),
        "ve_minutes": b.ve_minutes,
        "minutes": side(b.minutes_cap, u.watched_s / 60.0, r.minutes),
        "resets_at": (b.video_equivalents.is_some() || b.minutes_cap.is_some())
            .then(|| budget::next_midnight(&tz, now)),
        "window": {"restricted": !b.daily_windows.is_empty(), "open": next_open.is_none(),
            "opens_at": next_open, "closes_at": closes},
        "watching": live,
    }))
}

fn remaining_json(r: Remaining) -> Value {
    json!({"videos": r.videos, "minutes": r.minutes})
}

impl From<PlayError> for ApiError {
    fn from(e: PlayError) -> ApiError {
        use axum::http::StatusCode as S;
        match e {
            PlayError::NoGrant => ApiError::new(S::NOT_FOUND, "no such grant"),
            PlayError::Closed => ApiError::new(S::CONFLICT, "grant is closed"),
            PlayError::BadInput(w) => ApiError::new(S::BAD_REQUEST, w),
            PlayError::Db(m) => ApiError::internal(m),
        }
    }
}

fn push(
    state: &Arc<AppState>,
    name: &'static str,
    value: f64,
    unit: &'static str,
    labels: Vec<(&'static str, String)>,
) {
    if !state.sink.enabled() {
        return;
    }
    let state = state.clone();
    tokio::task::spawn_blocking(move || {
        let labels: Vec<(&str, &str)> = labels.iter().map(|(k, v)| (*k, v.as_str())).collect();
        state.sink.push_value(name, value, unit, &labels);
    });
}

async fn play_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Json(body): Json<PlayBody>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    let outcome = play(&conn, &dev, &body.youtube_id, unix_now())?;
    let (label, reason) = match &outcome {
        PlayOutcome::Granted { .. } => ("granted", ""),
        PlayOutcome::Refused { reason, .. } => ("refused", *reason),
    };
    push(
        &state,
        "cubby_plays",
        1.0,
        "1",
        vec![
            ("child", dev.child_id.clone()),
            ("outcome", label.into()),
            ("reason", reason.into()),
        ],
    );
    Ok(Json(match outcome {
        PlayOutcome::Granted {
            grant_id,
            remaining,
        } => json!({"grant_id": grant_id, "remaining": remaining_json(remaining)}),
        PlayOutcome::Refused { reason, next_start } => {
            json!({"refused": reason, "next_start": next_start})
        }
    }))
}

async fn beat_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path(id): Path<String>,
    Json(body): Json<BeatBody>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    let o = beat(&conn, &dev, &id, body.position_s, unix_now())?;
    if o.watched_delta_s > 0.0 {
        let l = vec![("child", dev.child_id.clone())];
        push(&state, "cubby_watch_seconds", o.watched_delta_s, "s", l);
    }
    if o.charge_delta > 0.0 {
        let l = vec![("child", dev.child_id.clone())];
        push(&state, "cubby_charge", o.charge_delta, "1", l);
    }
    let mut v = json!({"remaining": remaining_json(o.remaining), "stop": o.stop.is_some()});
    if let Some(grace) = o.stop {
        v["grace_s"] = json!(grace);
    }
    Ok(Json(v))
}

async fn end_route(
    State(state): State<Arc<AppState>>,
    Extension(ParentOf(family_id)): Extension<ParentOf>,
    Extension(dev): Extension<Device>,
    Path(id): Path<String>,
    Json(body): Json<EndBody>,
) -> Result<Json<Value>, ApiError> {
    let conn = open_own(&state, &family_id)?;
    end(&conn, &dev, &id, &body.reason, unix_now())?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_791_979_200; // 2026-10-14 12:00 UTC

    fn db() -> Connection {
        let mut c = Connection::open_in_memory().unwrap();
        cubby_db::family::migrate_family(&mut c).unwrap();
        c.execute_batch(
            "INSERT INTO children(id,name,avatar,tz) VALUES ('kid','K','','UTC');
             INSERT INTO devices(id,child_id,token_hash,paired_at) VALUES ('dev','kid',x'00',0);
             INSERT INTO videos(youtube_id,title,duration_s) VALUES ('v1','one',600),('v2','two',600),('nodur','x',NULL);
             INSERT INTO allow_policies(id,kind,youtube_id,added_at) VALUES ('p','channel','ch',0);
             INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES
               ('v1',NULL,'p','approved',0),('v2',NULL,'p','approved',0),('nodur',NULL,'p','approved',0);",
        )
        .unwrap();
        c
    }

    fn dev() -> Device {
        Device {
            id: "dev".into(),
            child_id: "kid".into(),
        }
    }

    /// A budget whose video equivalent is 10 minutes, so a fully watched 600 s video costs 1.
    fn budget_row(c: &Connection, ve: f64, cutoff: i64, windows: &str) {
        c.execute(
            "INSERT OR REPLACE INTO budgets(child_id,video_equivalents,minutes_cap,cutoff_mid_play,daily_windows,ve_minutes)
             VALUES ('kid',?1,NULL,?2,?3,10)",
            params![ve, cutoff, windows],
        )
        .unwrap();
    }

    #[test]
    fn usage_resets_at_local_midnight_not_24h_after_the_grant() {
        let c = db();
        c.execute("UPDATE children SET tz='America/Los_Angeles'", [])
            .unwrap();
        // 23:50 PDT on 2026-10-14 is 06:50 UTC on the 15th.
        let before = T0 + 18 * 3600 + 50 * 60;
        c.execute(
            "INSERT INTO play_grants(id,child_id,device_id,youtube_id,granted_at,last_beat_at,watched_s,charge)
             VALUES ('g',?1,'dev','v1',?2,?2,600,1.0)",
            params!["kid", before],
        )
        .unwrap();
        let la = "America/Los_Angeles";
        assert_eq!(usage(&c, "kid", la, before + 60).unwrap().charge, 1.0);
        assert_eq!(usage(&c, "kid", la, before + 20 * 60).unwrap().charge, 0.0);
        assert_eq!(
            progress_json(&c, "kid", before).unwrap()["resets_at"],
            Value::Null,
            "no limits, no reset"
        );
    }

    fn grant(c: &Connection, vid: &str, now: i64) -> String {
        match play(c, &dev(), vid, now).unwrap() {
            PlayOutcome::Granted { grant_id, .. } => grant_id,
            o => panic!("refused: {o:?}"),
        }
    }

    /// Plays honestly: a beat every 10 s with the position following the clock.
    fn watch(c: &Connection, id: &str, from: i64, seconds: i64) {
        for t in (10..=seconds).step_by(10) {
            beat(c, &dev(), id, t as f64, from + t).unwrap();
        }
    }

    fn charge_of(c: &Connection, id: &str) -> f64 {
        c.query_row("SELECT charge FROM play_grants WHERE id=?1", [id], |r| {
            r.get(0)
        })
        .unwrap()
    }

    #[test]
    fn quitting_at_95_percent_charges_0_95() {
        let c = db();
        budget_row(&c, 5.0, 0, "");
        let id = grant(&c, "v1", T0);
        watch(&c, &id, T0, 570);
        end(&c, &dev(), &id, "left", T0 + 571).unwrap();
        assert_eq!(charge_of(&c, &id), 0.95);
    }

    fn long_video(c: &Connection) {
        c.execute_batch(
            "INSERT INTO videos(youtube_id,title,duration_s) VALUES ('long','l',7200);
             INSERT INTO allowed_videos(youtube_id,child_id,policy_id,state,changed_at) VALUES ('long',NULL,'p','approved',0);",
        )
        .unwrap();
    }

    #[test]
    fn a_120_minute_video_watched_through_charges_8_at_the_default_unit() {
        let c = db();
        long_video(&c);
        let id = grant(&c, "long", T0);
        watch(&c, &id, T0, 7200);
        assert!((charge_of(&c, &id) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn a_35_minute_watch_at_the_default_unit_charges_2_33() {
        let c = db();
        long_video(&c);
        let id = grant(&c, "long", T0);
        watch(&c, &id, T0, 2100);
        assert!((charge_of(&c, &id) - 2.0 - 1.0 / 3.0).abs() < 1e-9);
    }

    #[test]
    fn a_20_second_watch_of_a_long_video_charges_nothing() {
        let c = db();
        long_video(&c);
        let id = grant(&c, "long", T0);
        watch(&c, &id, T0, 20);
        assert_eq!(charge_of(&c, &id), 0.0);
    }

    #[test]
    fn changing_the_unit_changes_the_next_charge_and_not_the_one_in_flight() {
        let c = db();
        long_video(&c);
        budget_row(&c, 50.0, 0, "");
        let first = grant(&c, "long", T0);
        c.execute("UPDATE budgets SET ve_minutes=5", []).unwrap();
        watch(&c, &first, T0, 600);
        assert!((charge_of(&c, &first) - 1.0).abs() < 1e-9);
        end(&c, &dev(), &first, "left", T0 + 601).unwrap();
        let next = grant(&c, "long", T0 + 700);
        watch(&c, &next, T0 + 700, 600);
        assert!((charge_of(&c, &next) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_grant_begun_before_the_change_is_not_rebilled() {
        let c = db();
        long_video(&c);
        let id = grant(&c, "long", T0);
        c.execute("UPDATE play_grants SET ve_s=NULL", []).unwrap();
        watch(&c, &id, T0, 3600);
        assert_eq!(charge_of(&c, &id), 0.5);
    }

    #[test]
    fn a_forward_scrub_to_the_end_is_free() {
        let c = db();
        let id = grant(&c, "v1", T0);
        beat(&c, &dev(), &id, 10.0, T0 + 10).unwrap();
        beat(&c, &dev(), &id, 590.0, T0 + 20).unwrap();
        let watched: f64 = c
            .query_row(
                "SELECT watched_s FROM play_grants WHERE id=?1",
                [&id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(watched, 10.0);
        assert_eq!(charge_of(&c, &id), 0.0);
    }

    #[test]
    fn a_short_play_is_free_and_spends_no_video() {
        let c = db();
        budget_row(&c, 1.0, 0, "");
        let id = grant(&c, "v1", T0);
        watch(&c, &id, T0, 20);
        end(&c, &dev(), &id, "left", T0 + 21).unwrap();
        assert_eq!(charge_of(&c, &id), 0.0);
        assert!(matches!(
            play(&c, &dev(), "v2", T0 + 30).unwrap(),
            PlayOutcome::Granted { .. }
        ));
    }

    #[test]
    fn a_spent_budget_refuses_the_next_start() {
        let c = db();
        budget_row(&c, 1.0, 0, "");
        let id = grant(&c, "v1", T0);
        watch(&c, &id, T0, 600);
        end(&c, &dev(), &id, "ended", T0 + 601).unwrap();
        assert_eq!(
            play(&c, &dev(), "v2", T0 + 700).unwrap(),
            PlayOutcome::Refused {
                reason: "out-of-videos",
                next_start: None
            }
        );
    }

    #[test]
    fn usage_ages_out_of_the_window() {
        let c = db();
        budget_row(&c, 1.0, 0, "");
        let id = grant(&c, "v1", T0);
        watch(&c, &id, T0, 600);
        end(&c, &dev(), &id, "ended", T0 + 601).unwrap();
        assert!(matches!(
            play(&c, &dev(), "v2", T0 + 86_401).unwrap(),
            PlayOutcome::Granted { .. }
        ));
    }

    #[test]
    fn an_unplayable_or_unpriced_video_is_not_allowed() {
        let c = db();
        for v in ["unknown", "nodur"] {
            assert_eq!(
                play(&c, &dev(), v, T0).unwrap(),
                PlayOutcome::Refused {
                    reason: "not-allowed",
                    next_start: None
                }
            );
        }
    }

    #[test]
    fn mid_play_overrun_stops_only_when_the_cutoff_is_on() {
        for (cutoff, expect) in [(0, None), (1, Some(60))] {
            let c = db();
            budget_row(&c, 1.0, cutoff, "");
            let id = grant(&c, "v1", T0);
            let mut last = None;
            for t in (10..=600).step_by(10) {
                last = beat(&c, &dev(), &id, t as f64, T0 + t).unwrap().stop;
            }
            assert_eq!(last, expect, "cutoff={cutoff}");
        }
    }

    #[test]
    fn a_play_outside_the_childs_window_is_refused_with_the_next_start() {
        let c = db();
        budget_row(&c, 5.0, 0, r#"[["14:00","18:00"]]"#);
        assert_eq!(
            play(&c, &dev(), "v1", T0).unwrap(),
            PlayOutcome::Refused {
                reason: "not-now",
                next_start: Some(T0 + 2 * 3600)
            }
        );
        c.execute("UPDATE children SET tz='America/Los_Angeles'", [])
            .unwrap();
        // 12:00 UTC is 05:00 PDT, still before 14:00.
        assert_eq!(
            play(&c, &dev(), "v1", T0).unwrap(),
            PlayOutcome::Refused {
                reason: "not-now",
                next_start: Some(T0 + 9 * 3600)
            }
        );
    }

    #[test]
    fn a_missed_end_times_out_after_sixty_seconds() {
        let c = db();
        let id = grant(&c, "v1", T0);
        beat(&c, &dev(), &id, 10.0, T0 + 10).unwrap();
        reap_stale(&c, T0 + 70).unwrap();
        let open: Option<i64> = c
            .query_row("SELECT ended_at FROM play_grants WHERE id=?1", [&id], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(open, None, "60 s without a beat is not yet a timeout");
        reap_stale(&c, T0 + 71).unwrap();
        let (ended, reason): (i64, String) = c
            .query_row(
                "SELECT ended_at, end_reason FROM play_grants WHERE id=?1",
                [&id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((ended, reason.as_str()), (T0 + 10, "timeout"));
        assert_eq!(beat(&c, &dev(), &id, 20.0, T0 + 80), Err(PlayError::Closed));
    }

    #[test]
    fn starting_another_play_leaves_the_first() {
        let c = db();
        let a = grant(&c, "v1", T0);
        let _b = grant(&c, "v2", T0 + 5);
        assert_eq!(end(&c, &dev(), &a, "left", T0 + 6), Err(PlayError::Closed));
    }

    #[test]
    fn another_childs_grant_is_not_found() {
        let c = db();
        c.execute_batch("INSERT INTO children(id,name,avatar,tz) VALUES ('sib','S','','UTC')")
            .unwrap();
        let id = grant(&c, "v1", T0);
        let other = Device {
            id: "dev".into(),
            child_id: "sib".into(),
        };
        assert_eq!(beat(&c, &other, &id, 1.0, T0 + 1), Err(PlayError::NoGrant));
    }

    #[test]
    fn end_rejects_an_unknown_reason() {
        let c = db();
        let id = grant(&c, "v1", T0);
        assert_eq!(
            end(&c, &dev(), &id, "boredom", T0),
            Err(PlayError::BadInput("reason"))
        );
    }
}

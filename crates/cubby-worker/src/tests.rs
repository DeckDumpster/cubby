use std::path::Path;

use cubby_yt::{DAILY_QUOTA_LIMIT, FixtureTransport};
use rusqlite::Connection;
use serde_json::json;

use super::*;

const T0: i64 = 1_800_000_000;

fn fixtures() -> FixtureTransport {
    FixtureTransport::from_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../cubby-yt/tests/fixtures"),
    )
    .unwrap()
}

fn db(policies: &[(&str, &str, &str)]) -> Connection {
    let mut c = Connection::open_in_memory().unwrap();
    cubby_db::family::migrate_family(&mut c).unwrap();
    c.execute(
        "INSERT INTO children(id, name, tz) VALUES('c1', 'C', 'UTC')",
        [],
    )
    .unwrap();
    for (id, kind, yt) in policies {
        c.execute(
            "INSERT INTO allow_policies VALUES(?,?,?,NULL,0,NULL)",
            [id, kind, yt],
        )
        .unwrap();
    }
    c
}

fn strings(c: &Connection, sql: &str) -> Vec<String> {
    c.prepare(sql)
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

fn used(c: &Connection) -> u32 {
    c.query_row("SELECT COALESCE(SUM(units),0) FROM yt_quota", [], |r| {
        r.get(0)
    })
    .unwrap()
}

#[test]
fn a_channel_policy_expands_into_rows_with_origin_and_state() {
    let c = db(&[("p1", "channel", "UCchan")]);
    let t = fixtures();
    let r = run_pass(&c, &t, T0, None);
    assert_eq!(r.outcome, "ok");
    assert_eq!(
        strings(
            &c,
            "SELECT youtube_id||':'||policy_id||':'||state FROM allowed_videos ORDER BY 1"
        ),
        [
            "v1:p1:auto-allowed",
            "v2:p1:auto-allowed",
            "v3:p1:auto-allowed",
            "v4:p1:auto-allowed",
            "v5:p1:auto-allowed"
        ]
    );
    assert_eq!(
        strings(
            &c,
            "SELECT youtube_id FROM videos WHERE unavailable_at IS NOT NULL"
        ),
        ["v4"]
    );
    assert_eq!(
        strings(&c, "SELECT youtube_id FROM playable ORDER BY 1"),
        ["v1", "v2", "v3", "v5"]
    );
    // channels.list, three playlist pages, one videos.list
    assert_eq!(used(&c), 5);
    assert_eq!(r.units.values().sum::<u32>(), 5);
}

#[test]
fn later_runs_page_only_until_a_known_id() {
    let c = db(&[("p1", "channel", "UCchan")]);
    let mut t = fixtures();
    run_pass(&c, &t, T0, None);
    t.add(
        "playlistItems",
        &[("part", "contentDetails"), ("playlistId", "UUchan"), ("maxResults", "50")],
        json!({"items":[{"contentDetails":{"videoId":"v6"}},{"contentDetails":{"videoId":"v1"}}],"nextPageToken":"T2"}),
    );
    t.add(
        "videos",
        &[("part", "snippet,contentDetails,status"), ("id", "v6")],
        json!({"items":[{"id":"v6","snippet":{"title":"Six","channelId":"UCchan","channelTitle":"Chan"},"contentDetails":{"duration":"PT1M"},"status":{"privacyStatus":"public"}}]}),
    );
    let before = t.calls().len();
    let r = run_pass(&c, &t, T0 + 900, None);
    assert_eq!(r.outcome, "ok");
    let calls = t.calls()[before..].to_vec();
    assert_eq!(
        calls,
        [
            "playlistItems?maxResults=50&part=contentDetails&playlistId=UUchan",
            "videos?id=v6&part=snippet,contentDetails,status"
        ]
    );
    assert_eq!(
        strings(
            &c,
            "SELECT youtube_id FROM allowed_videos WHERE policy_id='p1' ORDER BY 1"
        ),
        ["v1", "v2", "v3", "v4", "v5", "v6"]
    );
}

#[test]
fn a_video_policy_creates_an_approved_row() {
    let c = db(&[("a", "video", "pick"), ("b", "video", "gone")]);
    run_pass(&c, fixtures(), T0, None);
    assert_eq!(
        strings(
            &c,
            "SELECT youtube_id||':'||state FROM allowed_videos ORDER BY 1"
        ),
        ["gone:approved", "pick:approved"]
    );
    assert_eq!(strings(&c, "SELECT youtube_id FROM playable"), ["pick"]);
}

#[test]
fn a_video_that_turns_private_drops_out_of_playable_on_refresh() {
    let c = db(&[("p9", "video", "pick")]);
    let mut t = fixtures();
    run_pass(&c, &t, T0, None);
    t.add(
        "videos",
        &[("part", "snippet,contentDetails,status"), ("id", "pick")],
        json!({"items":[{"id":"pick","snippet":{"title":"Picked"},"contentDetails":{"duration":"PT1M"},"status":{"privacyStatus":"private"}}]}),
    );
    run_pass(&c, &t, T0 + 2 * DAY_S, None);
    assert!(strings(&c, "SELECT youtube_id FROM playable").is_empty());
    assert_eq!(
        strings(
            &c,
            "SELECT youtube_id FROM videos WHERE unavailable_at IS NOT NULL"
        ),
        ["pick"]
    );
}

#[test]
fn a_removed_policy_is_not_expanded() {
    let c = db(&[("p1", "channel", "UCchan")]);
    c.execute("UPDATE allow_policies SET removed_at = 1", [])
        .unwrap();
    let t = fixtures();
    run_pass(&c, &t, T0, None);
    assert!(t.calls().is_empty());
}

#[test]
fn the_pass_stops_at_the_daily_limit_and_commits_no_partial_expansion() {
    let c = db(&[("p1", "channel", "UCchan")]);
    let day = T0.div_euclid(DAY_S).to_string();
    c.execute(
        "INSERT INTO yt_quota VALUES(?, 'videos', ?)",
        rusqlite::params![day, DAILY_QUOTA_LIMIT - 2],
    )
    .unwrap();
    let t = fixtures();
    let r = run_pass(&c, &t, T0, None);
    assert_eq!(r.outcome, "quota");
    assert_eq!(used(&c), DAILY_QUOTA_LIMIT);
    assert_eq!(t.calls().len(), 2);
    assert!(strings(&c, "SELECT youtube_id FROM allowed_videos").is_empty());
    // Next day: the counter is fresh and the expansion completes.
    let r = run_pass(&c, &t, T0 + DAY_S, None);
    assert_eq!(r.outcome, "ok");
    assert_eq!(strings(&c, "SELECT youtube_id FROM playable").len(), 4);
}

#[test]
fn thumbnails_download_once_into_the_thumbs_dir() {
    let c = db(&[("p9", "video", "pick")]);
    let dir = std::env::temp_dir().join(format!("cubby-thumbs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let t = fixtures();
    run_pass(&c, &t, T0, Some(&dir));
    run_pass(&c, &t, T0 + 2 * DAY_S, Some(&dir));
    assert_eq!(
        t.calls().iter().filter(|k| k.starts_with("thumb ")).count(),
        1
    );
    let path: String = c
        .query_row(
            "SELECT thumb_path FROM videos WHERE youtube_id='pick'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(dir.join(&path).exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_api_key_disables_the_pass_without_touching_the_network() {
    let config = Config {
        data_dir: "/nonexistent".into(),
        api_key: String::new(),
        otlp_endpoint: String::new(),
    };
    assert_eq!(run_once(&config), "disabled");
}

#[test]
fn a_pass_pushes_run_seconds_with_its_outcome() {
    let f = cubby_otel::fake::FakeSink::start();
    let config = Config {
        data_dir: "/nonexistent".into(),
        api_key: String::new(),
        otlp_endpoint: f.addr.clone(),
    };
    run_once(&config);
    let got = f.wait_for("cubby_worker_run_seconds");
    assert_eq!(got[0].label("outcome"), Some("disabled"));
}

#[test]
fn a_stored_handle_policy_is_rewritten_to_its_channel_id_and_expands() {
    let c = db(&[("p1", "channel", "@Handle")]);
    let mut t = fixtures();
    t.add(
        "channels",
        &[("part", "snippet"), ("forHandle", "@Handle")],
        json!({"items": [{"id": "UCchan", "snippet": {"title": "Chan"}}]}),
    );
    let r = run_pass(&c, &t, T0, None);
    assert_eq!(r.outcome, "ok");
    assert_eq!(
        strings(&c, "SELECT youtube_id FROM allow_policies"),
        ["UCchan"]
    );
    assert_eq!(
        strings(&c, "SELECT label FROM policy_label WHERE policy_id='p1'"),
        ["@Handle"]
    );
    assert_eq!(
        strings(
            &c,
            "SELECT CAST(count(*) AS TEXT) FROM allowed_videos WHERE policy_id='p1'"
        ),
        ["5"]
    );
}

#[test]
fn a_handle_that_does_not_resolve_is_left_alone_and_reported() {
    let c = db(&[("p1", "channel", "@Ghost")]);
    let mut t = fixtures();
    t.add(
        "channels",
        &[("part", "snippet"), ("forHandle", "@Ghost")],
        json!({"items": []}),
    );
    let r = run_pass(&c, &t, T0, None);
    assert_eq!(r.outcome, "error");
    assert_eq!(
        strings(&c, "SELECT youtube_id FROM allow_policies"),
        ["@Ghost"]
    );
}

#[test]
fn a_handle_policy_whose_channel_is_already_allowed_is_dropped() {
    let c = db(&[("p1", "channel", "UCchan"), ("p2", "channel", "@Handle")]);
    let mut t = fixtures();
    t.add(
        "channels",
        &[("part", "snippet"), ("forHandle", "@Handle")],
        json!({"items": [{"id": "UCchan", "snippet": {"title": "Chan"}}]}),
    );
    run_pass(&c, &t, T0, None);
    assert_eq!(
        strings(&c, "SELECT id FROM allow_policies WHERE removed_at IS NULL"),
        ["p1"]
    );
}

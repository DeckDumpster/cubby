//! The child catalogue over HTTP, through the router the server builds for itself: a paired
//! device sees what the family allowed, and an unmounted `/api/*` path is a JSON 404.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_db::Registry;
use cubby_db::browse::fixture::{add_video, allow};
use cubby_db::identity as ident;
use cubby_server::{Config, app};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

async fn get(
    router: &axum::Router,
    path: &str,
    token: Option<&str>,
) -> (StatusCode, String, Value) {
    let mut r = Request::get(path);
    if let Some(t) = token {
        r = r.header("authorization", format!("Bearer {t}"));
    }
    let res = router
        .clone()
        .oneshot(r.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let ctype = res
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        ctype,
        serde_json::from_slice(&body).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn a_paired_device_browses_and_searches_what_was_allowed_and_unknown_api_paths_404() {
    let dir = std::env::temp_dir().join(format!("cubby-cat-{}-{}", std::process::id(), now()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>shell</html>").unwrap();
    let cfg = Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(dir.display().to_string()),
        "CUBBY_DATA_DIR" => Some(dir.join("data").display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(String::new()),
        _ => None,
    })
    .unwrap();
    let mut registry = Registry::open(&dir.join("data")).unwrap();
    let family_id = registry.create_family("mum@example.com").unwrap().id;
    let conn = registry.open_family(&family_id).unwrap();
    drop(registry);
    ident::add_child(&conn, "c1", "Ada", "Europe/London").unwrap();
    add_video(&conn, "v1", "Blue train", "Trains", "[]", 0);
    add_video(&conn, "v2", "Red bus", "Buses", "[]", 0);
    allow(&conn, "v1", "c1", "approved");
    let code = ident::new_pairing_code(&conn, "c1", now())
        .unwrap()
        .unwrap();
    let token = ident::redeem_pairing_code(&conn, &family_id, &code.code, None, now())
        .unwrap()
        .token;

    let router = app(&cfg).unwrap();

    let (s, ct, v) = get(&router, "/api/child/home", Some(&token)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(ct.starts_with("application/json"), "got {ct}");
    let all = v["sections"]
        .as_array()
        .unwrap()
        .iter()
        .find(|s| s["key"] == "all")
        .unwrap()["tiles"]
        .clone();
    let ids: Vec<&str> = all
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["youtube_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["v1"], "only the allowed video is listed");

    let (s, _, v) = get(&router, "/api/child/search?q=train", Some(&token)).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["tiles"][0]["youtube_id"], "v1");
    let (_, _, v) = get(&router, "/api/child/search?q=bus", Some(&token)).await;
    assert_eq!(
        v["tiles"].as_array().unwrap().len(),
        0,
        "unallowed video is not found"
    );

    let (s, _, v) = get(&router, "/api/child/next?after=v2", Some(&token)).await;
    assert_eq!(s, StatusCode::OK);
    assert!(v["tiles"].is_array());

    assert_eq!(
        get(&router, "/api/child/home", None).await.0,
        StatusCode::UNAUTHORIZED
    );

    let (s, ct, v) = get(&router, "/api/nope", Some(&token)).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert!(ct.starts_with("application/json"), "got {ct}");
    assert!(v["error"].is_string());
}

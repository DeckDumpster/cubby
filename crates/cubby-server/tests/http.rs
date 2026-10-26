use std::io::{Read, Write};
use std::net::TcpListener;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use cubby_server::{Config, app};
use http_body_util::BodyExt;
use tower::ServiceExt;

fn config(web_dir: &std::path::Path, otlp: &str) -> Config {
    Config::from_lookup(|k| match k {
        "CUBBY_WEB_DIR" => Some(web_dir.display().to_string()),
        "CUBBY_OTLP_ENDPOINT" => Some(otlp.to_string()),
        "CUBBY_DATA_DIR" => Some(web_dir.join("data").display().to_string()),
        _ => None,
    })
    .unwrap()
}

fn web_dir() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!(
        "cubby-web-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("index.html"), "<html>cubby shell</html>").unwrap();
    d
}

async fn get(app: axum::Router, path: &str) -> (StatusCode, String) {
    let res = app
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = res.status();
    let body = res.into_body().collect().await.unwrap().to_bytes();
    (status, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn healthz_returns_200_ok_true() {
    let (status, body) = get(app(&config(&web_dir(), "")).unwrap(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"ok":true}"#);
}

#[tokio::test]
async fn spa_shell_is_served_for_the_root_and_client_routes() {
    for path in ["/", "/parent", "/watch/deep/link"] {
        let (status, body) = get(app(&config(&web_dir(), "")).unwrap(), path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert!(body.contains("cubby shell"), "{path}");
    }
}

#[test]
fn defaults_port_8790_and_rejects_a_bad_port() {
    assert_eq!(Config::from_lookup(|_| None).unwrap().port, 8790);
    assert!(Config::from_lookup(|k| (k == "CUBBY_PORT").then(|| "x".into())).is_err());
}

#[tokio::test]
async fn a_request_sends_one_sample_to_the_otlp_sink() {
    let l = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = l.local_addr().unwrap().to_string();
    let sink = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut got = Vec::new();
        let mut buf = [0u8; 4096];
        while !got.windows(4).any(|w| w == b"\r\n\r\n") || got.len() < 200 {
            let n = s.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
        }
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        String::from_utf8_lossy(&got).into_owned()
    });
    let (status, _) = get(app(&config(&web_dir(), &addr)).unwrap(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    let got = tokio::task::spawn_blocking(move || sink.join().unwrap())
        .await
        .unwrap();
    assert!(got.starts_with("POST /opentelemetry/v1/metrics"), "{got}");
    assert!(got.contains("cubby_http_request_seconds"), "{got}");
    assert!(got.contains("/healthz"), "{got}");
}

#[tokio::test]
async fn an_empty_endpoint_sends_nothing() {
    let (status, _) = get(app(&config(&web_dir(), "")).unwrap(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
}

fn logged(config: &Config) -> (axum::Router, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
    let lines = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let keep = lines.clone();
    let log: cubby_server::LogSink = std::sync::Arc::new(move |l: &str| {
        keep.lock().unwrap().push(l.to_string());
    });
    (cubby_server::app_with_log(config, log).unwrap(), lines)
}

#[tokio::test]
async fn a_request_logs_one_line_with_route_and_status_and_no_secret() {
    let (router, lines) = logged(&config(&web_dir(), ""));
    let res = router
        .oneshot(
            Request::post("/api/parent/devices/dev-123/revoke")
                .header("authorization", "Bearer SECRET-TOKEN-abc")
                .header("cookie", "cubby_session=SECRET-JWT-xyz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let lines = lines.lock().unwrap();
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert!(
        line.contains(r#""route":"/api/parent/devices/{id}/revoke""#),
        "{line}"
    );
    assert!(line.contains(r#""status":401"#), "{line}");
    assert!(line.contains(r#""method":"POST""#), "{line}");
    assert!(line.contains("OIDC sign-in is not configured"), "{line}");
    for secret in ["SECRET", "dev-123"] {
        assert!(!line.contains(secret), "{secret} leaked: {line}");
    }
}

#[tokio::test]
async fn metrics_reach_a_fixture_otlp_endpoint_by_route_status_and_auth_reason() {
    let f = cubby_otel::fake::FakeSink::start();
    let app = app(&config(&web_dir(), &f.addr)).unwrap();
    let res = app
        .oneshot(Request::get("/api/child/me").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
    let reqs = tokio::task::spawn_blocking(move || {
        let reqs = f.wait_for("cubby_http_requests");
        let auth = f.wait_for("cubby_device_auth");
        (reqs, auth)
    })
    .await
    .unwrap();
    assert_eq!(reqs.0[0].label("route"), Some("/api/child/me"));
    assert_eq!(reqs.0[0].label("status"), Some("401"));
    assert_eq!(reqs.1[0].label("outcome"), Some("missing"));
}

async fn headers_of(path: &str) -> (StatusCode, axum::http::HeaderMap) {
    let res = app(&config(&web_dir(), ""))
        .unwrap()
        .oneshot(Request::get(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    (res.status(), res.headers().clone())
}

#[tokio::test]
async fn api_responses_are_never_cacheable_whatever_their_status() {
    for path in ["/api/child/home", "/api/parent/anything", "/api/nope"] {
        let (status, h) = headers_of(path).await;
        assert!(status.is_client_error(), "{path} {status}");
        assert_eq!(h["cache-control"], "no-store", "{path}");
    }
}

#[tokio::test]
async fn the_spa_shell_is_never_stored() {
    for path in ["/", "/index.html", "/watch/deep/link"] {
        let (status, h) = headers_of(path).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(h["cache-control"], "no-store", "{path}");
    }
}

#[tokio::test]
async fn an_unknown_static_looking_path_is_html_that_no_edge_may_store() {
    let (_, h) = headers_of("/thumbs-nope/x.jpg").await;
    assert_eq!(h["cache-control"], "no-store");
}

#[tokio::test]
async fn other_static_assets_keep_their_own_caching() {
    let dir = web_dir();
    std::fs::create_dir_all(dir.join("_app")).unwrap();
    std::fs::write(dir.join("_app/x.js"), "1").unwrap();
    let res = app(&config(&dir, ""))
        .unwrap()
        .oneshot(Request::get("/_app/x.js").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert!(res.headers().get("cache-control").is_none());
}

#[tokio::test]
async fn versioned_icons_are_immutable_and_the_manifest_is_cached_for_a_day() {
    let dir = web_dir();
    std::fs::create_dir_all(dir.join("assets/icons")).unwrap();
    std::fs::write(dir.join("assets/icons/icon-v1.png"), [0x89, b'P']).unwrap();
    std::fs::write(dir.join("assets/icons/favicon-v1.ico"), [0, 0, 1, 0]).unwrap();
    std::fs::write(dir.join("assets/icons/favicon-v1.svg"), "<svg/>").unwrap();
    std::fs::write(dir.join("assets/manifest.webmanifest"), "{}").unwrap();
    let router = app(&config(&dir, "")).unwrap();
    let cc = |path: &'static str| {
        let router = router.clone();
        async move {
            let res = router
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            (
                res.status(),
                res.headers()["cache-control"].to_str().unwrap().to_string(),
            )
        }
    };
    let (st, v) = cc("/assets/icons/icon-v1.png").await;
    assert_eq!(
        (st, v.as_str()),
        (StatusCode::OK, "public, max-age=31536000, immutable")
    );
    let (st, v) = cc("/assets/manifest.webmanifest").await;
    assert_eq!((st, v.as_str()), (StatusCode::OK, "public, max-age=86400"));
    let ctype = |path: &'static str| {
        let router = router.clone();
        async move {
            let res = router
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            res.headers()["content-type"].to_str().unwrap().to_string()
        }
    };
    assert_eq!(ctype("/assets/icons/icon-v1.png").await, "image/png");
    assert!(ctype("/assets/icons/favicon-v1.ico").await.contains("icon"));
    assert_eq!(ctype("/assets/icons/favicon-v1.svg").await, "image/svg+xml");
    assert!(
        ctype("/assets/manifest.webmanifest")
            .await
            .contains("manifest+json")
    );
    let (_, v) = cc("/assets/icons/missing.png").await;
    assert_eq!(v, "no-store");
}

#[tokio::test]
async fn a_thumb_the_worker_wrote_is_served_as_an_image_and_a_missing_one_is_404() {
    let dir = web_dir();
    std::fs::create_dir_all(dir.join("data/thumbs")).unwrap();
    std::fs::write(dir.join("data/thumbs/abc.jpg"), [0xff, 0xd8, 0xff, 0xd9]).unwrap();
    let router = app(&config(&dir, "")).unwrap();
    let res = router
        .clone()
        .oneshot(Request::get("/thumbs/abc.jpg").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/jpeg");
    assert!(
        res.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("max-age=")
    );
    let res = router
        .clone()
        .oneshot(
            Request::get("/thumbs/abc.jpg?v=7")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(res.headers()["content-type"], "image/jpeg");
    let (status, body) = get(router, "/thumbs/missing.jpg").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(!body.contains("cubby shell"));
}

#[tokio::test]
async fn the_native_player_page_is_html_no_store_with_a_youtube_frame_csp() {
    let dir = web_dir();
    std::fs::create_dir_all(dir.join("native")).unwrap();
    std::fs::write(dir.join("native/player.html"), "<html>native player</html>").unwrap();
    let res = app(&config(&dir, ""))
        .unwrap()
        .oneshot(Request::get("/native/player").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let h = res.headers();
    assert_eq!(h["cache-control"], "no-store");
    assert!(h["content-type"].to_str().unwrap().starts_with("text/html"));
    let csp = h["content-security-policy"].to_str().unwrap();
    assert!(
        csp.contains("frame-src https://www.youtube-nocookie.com"),
        "{csp}"
    );
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("native player"));
}

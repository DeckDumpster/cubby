use super::*;

#[derive(Default)]
struct Mem(u32, Vec<(String, u32)>);
impl Meter for Mem {
    fn used_today(&self) -> u32 {
        self.0
    }
    fn record(&mut self, call: &str, units: u32) {
        self.0 += units;
        self.1.push((call.into(), units));
    }
}

fn fixtures() -> FixtureTransport {
    FixtureTransport::from_dir(&Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures"))
        .unwrap()
}

fn ids(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn durations_parse_to_seconds() {
    assert_eq!(parse_duration("PT4M13S"), Some(253));
    assert_eq!(parse_duration("PT1H2M3S"), Some(3723));
    assert_eq!(parse_duration("PT45S"), Some(45));
    assert_eq!(parse_duration("P1DT1H"), Some(90000));
    assert_eq!(parse_duration("nonsense"), None);
}

#[test]
fn videos_list_splits_available_from_unavailable_and_costs_one_unit() {
    let mut c = Client::new(fixtures(), Mem::default());
    let r = c
        .videos_list(&ids(&["v1", "v2", "v3", "v4", "v5"]))
        .unwrap();
    let got: Vec<&str> = r.available.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(got, ["v1", "v2", "v3", "v5"]);
    assert_eq!(r.unavailable, ["v4"]);
    let v2 = &r.available[1];
    assert_eq!(v2.duration_s, 3723);
    assert_eq!(v2.title, "Two");
    assert_eq!(v2.channel_id, "UCchan");
    assert_eq!(v2.tags, ["t1", "t2"]);
    assert!(v2.made_for_kids);
    assert_eq!(
        v2.thumb_url.as_deref(),
        Some("https://i.ytimg.com/vi/v2/mqdefault.jpg")
    );
    assert_eq!(c.meter().1, [("videos".to_string(), 1)]);
}

#[test]
fn a_requested_id_missing_from_the_response_is_unavailable() {
    let mut c = Client::new(fixtures(), Mem::default());
    let r = c.videos_list(&ids(&["pick", "gone"])).unwrap();
    assert_eq!(r.available.len(), 1);
    assert_eq!(r.unavailable, ["gone"]);
}

#[test]
fn a_channel_resolves_to_its_uploads_playlist() {
    let mut c = Client::new(fixtures(), Mem::default());
    assert_eq!(
        c.uploads_playlist("UCchan").unwrap().as_deref(),
        Some("UUchan")
    );
    assert_eq!(c.uploads_playlist("UCgone").unwrap(), None);
}

#[test]
fn playlist_pages_carry_ids_and_a_next_token() {
    let mut c = Client::new(fixtures(), Mem::default());
    let p = c.playlist_items("UUchan", None).unwrap();
    assert_eq!(p.ids, ["v1", "v2"]);
    assert_eq!(p.next_page_token.as_deref(), Some("T2"));
    let p = c.playlist_items("UUchan", Some("T3")).unwrap();
    assert_eq!(p.ids, ["v5"]);
    assert_eq!(p.next_page_token, None);
}

#[test]
fn the_meter_stops_the_client_at_the_daily_limit() {
    let mut c = Client::new(fixtures(), Mem(DAILY_QUOTA_LIMIT - 1, vec![]));
    c.playlist_items("UUchan", None).unwrap();
    assert_eq!(c.meter().used_today(), DAILY_QUOTA_LIMIT);
    assert_eq!(
        c.playlist_items("UUchan", Some("T2")),
        Err(YtError::QuotaExhausted)
    );
    assert_eq!(c.videos_list(&ids(&["v5"])), Err(YtError::QuotaExhausted));
    assert_eq!(c.meter().used_today(), DAILY_QUOTA_LIMIT);
}

#[test]
fn an_unrecorded_request_is_an_error_not_a_network_call() {
    let mut c = Client::new(fixtures(), Mem::default());
    assert!(matches!(
        c.videos_list(&ids(&["nope"])),
        Err(YtError::Http(_))
    ));
}

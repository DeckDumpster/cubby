//! YouTube Data API v3 client with a quota meter. Transport is a trait so tests replay recorded
//! fixtures and never reach the network.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use serde_json::Value;

/// The most quota units the worker spends in a day (the API's own ceiling is 10,000).
pub const DAILY_QUOTA_LIMIT: u32 = 8_000;
/// Quota units a `videos.list` call costs.
pub const VIDEOS_LIST_COST: u32 = 1;
/// Quota units a `channels.list` call costs.
pub const CHANNELS_LIST_COST: u32 = 1;
/// Quota units a `playlistItems.list` call costs.
pub const PLAYLIST_ITEMS_COST: u32 = 1;
/// Ids a single `videos.list` call and a single playlist page carry.
pub const BATCH: usize = 50;

#[derive(Debug, PartialEq)]
pub enum YtError {
    QuotaExhausted,
    Http(String),
    Parse(String),
}

impl std::fmt::Display for YtError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            YtError::QuotaExhausted => write!(f, "daily quota exhausted"),
            YtError::Http(e) => write!(f, "http: {e}"),
            YtError::Parse(e) => write!(f, "parse: {e}"),
        }
    }
}

impl std::error::Error for YtError {}

pub trait Transport {
    /// GET `endpoint` (e.g. `videos`) with `params`; the API key is the transport's business.
    fn get_json(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Value, YtError>;
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, YtError>;
}

impl<T: Transport + ?Sized> Transport for &T {
    fn get_json(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Value, YtError> {
        (**self).get_json(endpoint, params)
    }
    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, YtError> {
        (**self).get_bytes(url)
    }
}

/// The day's quota counter. The worker backs it with the family database.
pub trait Meter {
    fn used_today(&self) -> u32;
    fn record(&mut self, call: &str, units: u32);
}

#[derive(Debug, Clone, PartialEq)]
pub struct Video {
    pub id: String,
    pub title: String,
    pub channel_id: String,
    pub channel_title: String,
    pub tags: Vec<String>,
    pub duration_s: u64,
    pub thumb_url: Option<String>,
    pub made_for_kids: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct VideosResult {
    pub available: Vec<Video>,
    /// Requested ids that came back missing, private or otherwise not viewable.
    pub unavailable: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct ItemsPage {
    pub ids: Vec<String>,
    pub next_page_token: Option<String>,
}

pub struct Client<T, M> {
    transport: T,
    meter: M,
}

impl<T: Transport, M: Meter> Client<T, M> {
    pub fn new(transport: T, meter: M) -> Self {
        Client { transport, meter }
    }

    pub fn meter(&self) -> &M {
        &self.meter
    }

    /// Refuses the call when it would take the day past the limit; otherwise runs it and records
    /// its cost under `call`.
    fn call(&mut self, call: &str, cost: u32, params: &[(&str, &str)]) -> Result<Value, YtError> {
        if self.meter.used_today() + cost > DAILY_QUOTA_LIMIT {
            return Err(YtError::QuotaExhausted);
        }
        let r = self.transport.get_json(call, params);
        self.meter.record(call, cost);
        r
    }

    pub fn videos_list(&mut self, ids: &[String]) -> Result<VideosResult, YtError> {
        let joined = ids.join(",");
        let body = self.call(
            "videos",
            VIDEOS_LIST_COST,
            &[("part", "snippet,contentDetails,status"), ("id", &joined)],
        )?;
        let mut out = VideosResult::default();
        let mut seen = std::collections::HashSet::new();
        for item in body["items"].as_array().into_iter().flatten() {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            seen.insert(id.to_string());
            match parse_video(id, item) {
                Some(v) => out.available.push(v),
                None => out.unavailable.push(id.to_string()),
            }
        }
        for id in ids {
            if !seen.contains(id) {
                out.unavailable.push(id.clone());
            }
        }
        Ok(out)
    }

    pub fn uploads_playlist(&mut self, channel_id: &str) -> Result<Option<String>, YtError> {
        let body = self.call(
            "channels",
            CHANNELS_LIST_COST,
            &[("part", "contentDetails"), ("id", channel_id)],
        )?;
        Ok(
            body["items"][0]["contentDetails"]["relatedPlaylists"]["uploads"]
                .as_str()
                .map(str::to_string),
        )
    }

    pub fn playlist_items(
        &mut self,
        playlist_id: &str,
        page_token: Option<&str>,
    ) -> Result<ItemsPage, YtError> {
        let mut params = vec![
            ("part", "contentDetails"),
            ("playlistId", playlist_id),
            ("maxResults", "50"),
        ];
        if let Some(t) = page_token {
            params.push(("pageToken", t));
        }
        let body = self.call("playlistItems", PLAYLIST_ITEMS_COST, &params)?;
        Ok(ItemsPage {
            ids: body["items"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|i| i["contentDetails"]["videoId"].as_str())
                .map(str::to_string)
                .collect(),
            next_page_token: body["nextPageToken"].as_str().map(str::to_string),
        })
    }

    pub fn thumbnail(&self, url: &str) -> Result<Vec<u8>, YtError> {
        self.transport.get_bytes(url)
    }
}

/// Parses an ISO 8601 duration as YouTube reports it (`PT1H2M3S`) to whole seconds.
pub fn parse_duration(s: &str) -> Option<u64> {
    let rest = s.strip_prefix('P')?;
    let (date, time) = rest.split_once('T').unwrap_or((rest, ""));
    let mut total = 0u64;
    for (part, units) in [
        (date, [('D', 86_400u64)].as_slice()),
        (time, [('H', 3600), ('M', 60), ('S', 1)].as_slice()),
    ] {
        let mut num = String::new();
        for ch in part.chars() {
            if ch.is_ascii_digit() {
                num.push(ch);
            } else {
                let (_, mult) = units.iter().find(|(u, _)| *u == ch)?;
                total += num.parse::<u64>().ok()? * mult;
                num.clear();
            }
        }
        if !num.is_empty() {
            return None;
        }
    }
    Some(total)
}

/// `None` when the video is not viewable: private, or rejected/failed on upload.
fn parse_video(id: &str, item: &Value) -> Option<Video> {
    let status = &item["status"];
    if status["privacyStatus"].as_str() == Some("private")
        || matches!(
            status["uploadStatus"].as_str(),
            Some("rejected" | "failed" | "deleted")
        )
    {
        return None;
    }
    let snip = &item["snippet"];
    let thumbs = &snip["thumbnails"];
    let thumb_url = ["medium", "default", "high"]
        .iter()
        .find_map(|k| thumbs[k]["url"].as_str())
        .map(str::to_string);
    Some(Video {
        id: id.to_string(),
        title: snip["title"].as_str()?.to_string(),
        channel_id: snip["channelId"].as_str().unwrap_or_default().to_string(),
        channel_title: snip["channelTitle"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        tags: snip["tags"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|t| t.as_str().map(str::to_string))
            .collect(),
        duration_s: parse_duration(item["contentDetails"]["duration"].as_str()?)?,
        thumb_url,
        made_for_kids: status["madeForKids"].as_bool().unwrap_or(false),
    })
}

fn request_key(endpoint: &str, params: &[(&str, &str)]) -> String {
    let mut p: Vec<String> = params
        .iter()
        .filter(|(k, _)| *k != "key")
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    p.sort();
    format!("{endpoint}?{}", p.join("&"))
}

/// Replays recorded responses. A fixture file is JSON
/// `{"endpoint": .., "params": {..}, "response": {..}}`; the request key is the endpoint and the
/// sorted params, without the API key.
#[derive(Default)]
pub struct FixtureTransport {
    responses: HashMap<String, Value>,
    calls: RefCell<Vec<String>>,
}

impl FixtureTransport {
    pub fn from_dir(dir: &Path) -> std::io::Result<Self> {
        let mut t = FixtureTransport::default();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.extension().is_none_or(|e| e != "json") {
                continue;
            }
            let doc: Value = serde_json::from_slice(&std::fs::read(&path)?)
                .map_err(|e| std::io::Error::other(format!("{}: {e}", path.display())))?;
            let params: Vec<(String, String)> = doc["params"]
                .as_object()
                .into_iter()
                .flatten()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect();
            let refs: Vec<(&str, &str)> = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            t.add(
                doc["endpoint"].as_str().unwrap_or_default(),
                &refs,
                doc["response"].clone(),
            );
        }
        Ok(t)
    }

    /// Records (or replaces) the response for a request.
    pub fn add(&mut self, endpoint: &str, params: &[(&str, &str)], response: Value) {
        self.responses
            .insert(request_key(endpoint, params), response);
    }

    /// The request keys served so far, in order.
    pub fn calls(&self) -> Vec<String> {
        self.calls.borrow().clone()
    }
}

impl Transport for FixtureTransport {
    fn get_json(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Value, YtError> {
        let key = request_key(endpoint, params);
        self.calls.borrow_mut().push(key.clone());
        self.responses
            .get(&key)
            .cloned()
            .ok_or_else(|| YtError::Http(format!("no recorded fixture for {key}")))
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, YtError> {
        self.calls.borrow_mut().push(format!("thumb {url}"));
        Ok(format!("fixture-thumb:{url}").into_bytes())
    }
}

/// The real transport: HTTPS to the Data API with `key`, under a deadline.
pub struct HttpTransport {
    key: String,
    base: String,
    agent: ureq::Agent,
}

/// Where the Data API lives unless `CUBBY_YT_BASE_URL` says otherwise.
pub const DEFAULT_BASE_URL: &str = "https://www.googleapis.com/youtube/v3";

impl HttpTransport {
    /// Talks to the Data API, or to `CUBBY_YT_BASE_URL` when set: the end-to-end test points the
    /// server and worker binaries at a local fake YouTube that way.
    pub fn new(key: &str) -> Self {
        let base = std::env::var("CUBBY_YT_BASE_URL")
            .ok()
            .map(|b| b.trim().trim_end_matches('/').to_string())
            .filter(|b| !b.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        HttpTransport {
            key: key.to_string(),
            base,
            agent: ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(20))
                .build(),
        }
    }
}

impl Transport for HttpTransport {
    fn get_json(&self, endpoint: &str, params: &[(&str, &str)]) -> Result<Value, YtError> {
        let mut req = self
            .agent
            .get(&format!("{}/{endpoint}", self.base))
            .query("key", &self.key);
        for (k, v) in params {
            req = req.query(k, v);
        }
        match req.call() {
            Ok(r) => {
                serde_json::from_reader(r.into_reader()).map_err(|e| YtError::Parse(e.to_string()))
            }
            Err(ureq::Error::Status(403, _)) => Err(YtError::QuotaExhausted),
            Err(e) => Err(YtError::Http(e.to_string())),
        }
    }

    fn get_bytes(&self, url: &str) -> Result<Vec<u8>, YtError> {
        use std::io::Read;
        let r = self
            .agent
            .get(url)
            .call()
            .map_err(|e| YtError::Http(e.to_string()))?;
        let mut buf = Vec::new();
        r.into_reader()
            .take(5_000_000)
            .read_to_end(&mut buf)
            .map_err(|e| YtError::Http(e.to_string()))?;
        Ok(buf)
    }
}

#[cfg(test)]
mod tests;

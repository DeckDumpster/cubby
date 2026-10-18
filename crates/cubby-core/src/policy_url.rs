//! Turns a pasted YouTube URL into the allow-list policy it names.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyKind {
    Video,
    Channel,
    Playlist,
}

impl PolicyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            PolicyKind::Video => "video",
            PolicyKind::Channel => "channel",
            PolicyKind::Playlist => "playlist",
        }
    }
}

/// What a pasted URL resolves to: a kind and the YouTube id (a channel `@handle` keeps its `@`
/// until the worker resolves it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub kind: PolicyKind,
    pub youtube_id: String,
}

fn id_ok(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v)
}

pub fn resolve_url(input: &str) -> Option<Resolved> {
    let s = input.trim();
    let s = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"))
        .unwrap_or(s);
    let s = s.split('#').next()?;
    let (host_path, query) = s.split_once('?').unwrap_or((s, ""));
    let (host, path) = host_path.split_once('/').unwrap_or((host_path, ""));
    let host = host.to_ascii_lowercase();
    let path = path.trim_matches('/');
    let make = |kind, id: &str| {
        id_ok(id).then(|| Resolved {
            kind,
            youtube_id: id.to_string(),
        })
    };
    match host.as_str() {
        "youtu.be" => make(PolicyKind::Video, path.split('/').next()?),
        "youtube.com"
        | "www.youtube.com"
        | "m.youtube.com"
        | "music.youtube.com"
        | "youtube-nocookie.com"
        | "www.youtube-nocookie.com" => {
            let mut parts = path.split('/');
            match parts.next()? {
                "watch" => make(PolicyKind::Video, query_param(query, "v")?),
                "playlist" => make(PolicyKind::Playlist, query_param(query, "list")?),
                "shorts" | "live" | "embed" | "v" => make(PolicyKind::Video, parts.next()?),
                "channel" => make(PolicyKind::Channel, parts.next()?),
                h if h.starts_with('@') && id_ok(&h[1..]) => Some(Resolved {
                    kind: PolicyKind::Channel,
                    youtube_id: h.to_string(),
                }),
                _ => None,
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(input: &str) -> Option<(PolicyKind, String)> {
        resolve_url(input).map(|r| (r.kind, r.youtube_id))
    }

    #[test]
    fn a_watch_url_is_a_video() {
        assert_eq!(
            r("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=5s"),
            Some((PolicyKind::Video, "dQw4w9WgXcQ".into()))
        );
    }

    #[test]
    fn a_short_url_is_a_video() {
        assert_eq!(
            r("https://youtu.be/dQw4w9WgXcQ?si=abc"),
            Some((PolicyKind::Video, "dQw4w9WgXcQ".into()))
        );
    }

    #[test]
    fn shorts_live_embed_and_v_paths_are_videos() {
        for u in [
            "https://www.youtube.com/shorts/dQw4w9WgXcQ",
            "https://m.youtube.com/shorts/dQw4w9WgXcQ?si=abc",
            "https://youtube.com/shorts/dQw4w9WgXcQ/",
            "https://www.youtube.com/live/dQw4w9WgXcQ?si=abc&t=30",
            "https://www.youtube.com/embed/dQw4w9WgXcQ?start=5",
            "https://www.youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://youtube-nocookie.com/embed/dQw4w9WgXcQ",
            "https://www.youtube.com/v/dQw4w9WgXcQ",
            "https://m.youtube.com/watch?v=dQw4w9WgXcQ&si=x&t=10s",
            "https://youtu.be/dQw4w9WgXcQ?t=10",
        ] {
            assert_eq!(r(u), Some((PolicyKind::Video, "dQw4w9WgXcQ".into())), "{u}");
        }
    }

    #[test]
    fn a_form_without_an_id_does_not_resolve() {
        for bad in [
            "https://www.youtube.com/shorts/",
            "https://www.youtube.com/live",
            "https://www.youtube.com/embed/",
        ] {
            assert_eq!(r(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_handle_is_a_channel() {
        assert_eq!(
            r("https://www.youtube.com/@BlueyOfficial/videos"),
            Some((PolicyKind::Channel, "@BlueyOfficial".into()))
        );
    }

    #[test]
    fn a_channel_url_is_a_channel() {
        assert_eq!(
            r("youtube.com/channel/UCabc_123-x"),
            Some((PolicyKind::Channel, "UCabc_123-x".into()))
        );
    }

    #[test]
    fn a_playlist_url_is_a_playlist() {
        assert_eq!(
            r("https://www.youtube.com/playlist?list=PL12345"),
            Some((PolicyKind::Playlist, "PL12345".into()))
        );
    }

    #[test]
    fn a_watch_url_inside_a_playlist_is_the_video() {
        assert_eq!(
            r("https://www.youtube.com/watch?v=abcdefghijk&list=PL1"),
            Some((PolicyKind::Video, "abcdefghijk".into()))
        );
    }

    #[test]
    fn other_hosts_and_junk_do_not_resolve() {
        for bad in [
            "",
            "hello",
            "https://example.com/watch?v=abcdefghijk",
            "https://youtube.com.evil.com/watch?v=abcdefghijk",
            "https://www.youtube.com/watch",
            "https://www.youtube.com/",
            "https://www.youtube.com/watch?v=a b",
        ] {
            assert_eq!(r(bad), None, "{bad}");
        }
    }
}

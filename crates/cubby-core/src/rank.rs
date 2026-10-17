//! The ranker: orders candidates the server already knows are playable.

use std::collections::HashSet;

pub type VideoId = String;

/// What the ranker may know about the child.
#[derive(Clone, Debug, Default)]
pub struct ChildCtx {
    pub watched: HashSet<VideoId>,
    pub liked_channels: HashSet<String>,
    /// Unix seconds.
    pub now: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct VideoMeta {
    pub id: VideoId,
    pub channel_id: String,
    pub tags: Vec<String>,
    /// Unix seconds of the upload.
    pub published_at: i64,
}

pub trait Ranker {
    fn rank(
        &self,
        child: &ChildCtx,
        current: Option<&VideoMeta>,
        candidates: &[VideoMeta],
    ) -> Vec<VideoId>;
}

const YEAR_S: f64 = 365.0 * 86_400.0;

/// The v1 score: a weighted sum of channel, tag, novelty, liked-channel and recency signals.
pub struct V1Ranker;

impl V1Ranker {
    pub fn score(child: &ChildCtx, current: Option<&VideoMeta>, v: &VideoMeta) -> f64 {
        let mut s = 0.0;
        if let Some(cur) = current {
            if cur.channel_id == v.channel_id {
                s += 3.0;
            }
            let shared = v.tags.iter().filter(|t| cur.tags.contains(t)).count();
            s += shared.min(3) as f64;
        }
        if !child.watched.contains(&v.id) {
            s += 2.0;
        }
        if child.liked_channels.contains(&v.channel_id) {
            s += 2.0;
        }
        let age = (child.now - v.published_at).max(0) as f64;
        s + (1.0 - age / YEAR_S).clamp(0.0, 1.0)
    }
}

impl Ranker for V1Ranker {
    fn rank(
        &self,
        child: &ChildCtx,
        current: Option<&VideoMeta>,
        candidates: &[VideoMeta],
    ) -> Vec<VideoId> {
        let mut scored: Vec<(f64, &VideoMeta)> = candidates
            .iter()
            .filter(|v| current.is_none_or(|c| c.id != v.id))
            .map(|v| (Self::score(child, current, v), v))
            .collect();
        scored.sort_by(|a, b| b.0.total_cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
        scored.into_iter().map(|(_, v)| v.id.clone()).collect()
    }
}

/// Runs `ranker` and keeps only ids that were in `candidates`, once each, in ranker order, so a
/// buggy ranker cannot leak a video outside the set the server passed in.
pub fn rank_checked(
    ranker: &dyn Ranker,
    child: &ChildCtx,
    current: Option<&VideoMeta>,
    candidates: &[VideoMeta],
) -> Vec<VideoId> {
    let allowed: HashSet<&str> = candidates.iter().map(|v| v.id.as_str()).collect();
    let mut seen = HashSet::new();
    ranker
        .rank(child, current, candidates)
        .into_iter()
        .filter(|id| allowed.contains(id.as_str()) && seen.insert(id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    const NOW: i64 = 1_800_000_000;

    fn meta(id: &str, channel: &str, tags: &[&str], age_days: i64) -> VideoMeta {
        VideoMeta {
            id: id.into(),
            channel_id: channel.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            published_at: NOW - age_days * 86_400,
        }
    }

    fn ctx() -> ChildCtx {
        ChildCtx {
            now: NOW,
            ..Default::default()
        }
    }

    #[test]
    fn same_channel_outranks_a_newer_stranger() {
        let cur = meta("cur", "a", &[], 1);
        let c = [meta("other", "b", &[], 0), meta("same", "a", &[], 300)];
        assert_eq!(V1Ranker.rank(&ctx(), Some(&cur), &c), ["same", "other"]);
    }

    #[test]
    fn shared_tags_count_up_to_three() {
        let cur = meta("cur", "a", &["1", "2", "3", "4"], 1);
        let v = meta("v", "b", &["1", "2", "3", "4"], 400);
        let base = V1Ranker::score(&ctx(), Some(&cur), &meta("w", "b", &[], 400));
        assert_eq!(V1Ranker::score(&ctx(), Some(&cur), &v) - base, 3.0);
    }

    #[test]
    fn unwatched_and_liked_channel_add_two_each() {
        let mut child = ctx();
        let v = meta("v", "b", &[], 400);
        let none = V1Ranker::score(&child, None, &v);
        assert_eq!(none, 2.0);
        child.watched.insert("v".into());
        assert_eq!(V1Ranker::score(&child, None, &v), 0.0);
        child.liked_channels.insert("b".into());
        assert_eq!(V1Ranker::score(&child, None, &v), 2.0);
    }

    #[test]
    fn recency_runs_from_one_to_zero_over_a_year() {
        let mut child = ctx();
        child.watched.insert("v".into());
        let fresh = V1Ranker::score(&child, None, &meta("v", "b", &[], 0));
        let half = V1Ranker::score(&child, None, &meta("v", "b", &[], 183));
        let old = V1Ranker::score(&child, None, &meta("v", "b", &[], 900));
        assert_eq!(fresh, 1.0);
        assert!((half - 0.5).abs() < 0.01);
        assert_eq!(old, 0.0);
    }

    #[test]
    fn the_current_video_is_never_recommended() {
        let cur = meta("cur", "a", &[], 1);
        let c = [cur.clone(), meta("x", "a", &[], 1)];
        assert_eq!(V1Ranker.rank(&ctx(), Some(&cur), &c), ["x"]);
    }

    struct Leaky;
    impl Ranker for Leaky {
        fn rank(&self, _: &ChildCtx, _: Option<&VideoMeta>, c: &[VideoMeta]) -> Vec<VideoId> {
            let mut out = vec!["foreign".to_string()];
            out.extend(c.iter().map(|v| v.id.clone()));
            out.push("foreign2".into());
            out.extend(c.iter().map(|v| v.id.clone()));
            out
        }
    }

    #[test]
    fn rank_checked_drops_foreign_and_duplicate_ids() {
        let c = [meta("a", "x", &[], 1), meta("b", "x", &[], 1)];
        assert_eq!(rank_checked(&Leaky, &ctx(), None, &c), ["a", "b"]);
    }

    fn arb_meta() -> impl Strategy<Value = VideoMeta> {
        (
            "[a-f]{1,2}",
            "[a-c]",
            proptest::collection::vec("[a-d]", 0..5),
            0i64..2_000,
        )
            .prop_map(|(id, channel_id, tags, age)| VideoMeta {
                id,
                channel_id,
                tags,
                published_at: NOW - age * 86_400,
            })
    }

    proptest! {
        #[test]
        fn no_output_id_is_outside_the_candidates(
            cands in proptest::collection::vec(arb_meta(), 0..20),
            cur in proptest::option::of(arb_meta()),
        ) {
            let ids: HashSet<&str> = cands.iter().map(|v| v.id.as_str()).collect();
            for r in [&Leaky as &dyn Ranker, &V1Ranker] {
                for id in rank_checked(r, &ctx(), cur.as_ref(), &cands) {
                    prop_assert!(ids.contains(id.as_str()));
                }
            }
        }
    }
}

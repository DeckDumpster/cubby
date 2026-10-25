-- Per-policy filter rules for a channel or playlist: a video that fails them is not allowed
-- through that policy. Title words are stored lowercased; `has` = at least one must appear,
-- `lacks` = none may appear.
CREATE TABLE policy_rules(
    policy_id TEXT PRIMARY KEY REFERENCES allow_policies(id),
    exclude_shorts INTEGER NOT NULL DEFAULT 0,
    min_s INTEGER,
    max_s INTEGER
);

CREATE TABLE policy_rule_words(
    policy_id TEXT NOT NULL REFERENCES allow_policies(id),
    mode TEXT NOT NULL CHECK(mode IN ('has','lacks')),
    word TEXT NOT NULL,
    PRIMARY KEY(policy_id, mode, word)
);

-- (policy, video) pairs where the video fails the policy's rules. A duration rule fails a video
-- whose duration is not yet known. A Short is a video of 60 seconds or less: the Data API gives
-- no Shorts flag, aspect ratio or URL form, so duration is the only signal Cubby fetches.
-- Evaluated at read time, so every current video and every later upload is judged the same way.
CREATE VIEW rule_failed(policy_id, youtube_id) AS
SELECT r.policy_id, v.youtube_id
FROM policy_rules r
CROSS JOIN videos v
WHERE ((r.exclude_shorts=1 OR r.min_s IS NOT NULL OR r.max_s IS NOT NULL) AND v.duration_s IS NULL)
   OR (r.exclude_shorts=1 AND v.duration_s <= 60)
   OR (r.min_s IS NOT NULL AND v.duration_s < r.min_s)
   OR (r.max_s IS NOT NULL AND v.duration_s > r.max_s)
   OR (EXISTS (SELECT 1 FROM policy_rule_words w WHERE w.policy_id=r.policy_id AND w.mode='has')
       AND NOT EXISTS (SELECT 1 FROM policy_rule_words w WHERE w.policy_id=r.policy_id
            AND w.mode='has' AND instr(lower(v.title), w.word) > 0))
   OR EXISTS (SELECT 1 FROM policy_rule_words w WHERE w.policy_id=r.policy_id
            AND w.mode='lacks' AND instr(lower(v.title), w.word) > 0);

DROP VIEW playable;
CREATE VIEW playable(child_id, youtube_id) AS
SELECT c.id, v.youtube_id
FROM children c
CROSS JOIN videos v
WHERE v.unavailable_at IS NULL
  AND EXISTS (
    SELECT 1 FROM allowed_videos a
    JOIN allow_policies p ON p.id = a.policy_id
    WHERE a.youtube_id = v.youtube_id
      AND (a.child_id IS NULL OR a.child_id = c.id)
      AND a.state IN ('auto-allowed','approved')
      AND p.removed_at IS NULL
      AND NOT EXISTS (SELECT 1 FROM rule_failed f
                      WHERE f.policy_id = a.policy_id AND f.youtube_id = a.youtube_id))
  AND NOT EXISTS (
    SELECT 1 FROM allowed_videos b
    WHERE b.youtube_id = v.youtube_id
      AND (b.child_id IS NULL OR b.child_id = c.id)
      AND b.state = 'blocked');

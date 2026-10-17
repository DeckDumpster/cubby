CREATE TABLE children(
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    avatar TEXT NOT NULL DEFAULT '',
    tz TEXT NOT NULL
);

CREATE TABLE devices(
    id TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    token_hash BLOB NOT NULL UNIQUE,
    label TEXT NOT NULL DEFAULT '',
    paired_at INTEGER NOT NULL,
    last_seen INTEGER,
    revoked_at INTEGER
);

CREATE TABLE pairing_codes(
    code TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    expires_at INTEGER NOT NULL,
    used_at INTEGER
);

CREATE TABLE allow_policies(
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL CHECK(kind IN ('video','channel','playlist')),
    youtube_id TEXT NOT NULL,
    child_id TEXT,  -- NULL = all children
    added_at INTEGER NOT NULL,
    removed_at INTEGER
);

CREATE TABLE allowed_videos(
    youtube_id TEXT NOT NULL,
    child_id TEXT,  -- NULL = all children
    policy_id TEXT NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('auto-allowed','approved','blocked')),
    changed_at INTEGER NOT NULL,
    PRIMARY KEY(youtube_id, child_id, policy_id)
);
CREATE INDEX allowed_videos_by_policy ON allowed_videos(policy_id);

CREATE TABLE videos(
    youtube_id TEXT PRIMARY KEY,
    title TEXT NOT NULL DEFAULT '',
    channel_id TEXT,
    channel_title TEXT NOT NULL DEFAULT '',
    tags TEXT NOT NULL DEFAULT '[]',  -- json
    duration_s INTEGER,
    thumb_path TEXT,
    made_for_kids INTEGER,
    fetched_at INTEGER,
    unavailable_at INTEGER
);

CREATE VIRTUAL TABLE videos_fts USING fts5(title, channel_title, tags, content='videos');

CREATE TABLE budgets(
    child_id TEXT PRIMARY KEY REFERENCES children(id),
    video_equivalents REAL,
    window_s INTEGER,
    minutes_cap INTEGER,
    cutoff_mid_play INTEGER NOT NULL DEFAULT 0,
    daily_windows TEXT  -- json [["07:00","19:00"]] in the child's tz
);

CREATE TABLE play_grants(
    id TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    device_id TEXT NOT NULL REFERENCES devices(id),
    youtube_id TEXT NOT NULL,
    granted_at INTEGER NOT NULL,
    last_beat_at INTEGER,
    max_position_s REAL,
    watched_s REAL,
    charge REAL,
    ended_at INTEGER,
    end_reason TEXT
);

CREATE TABLE requests(
    id TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    kind TEXT NOT NULL CHECK(kind IN ('like','more-from-channel')),
    youtube_id TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    resolved_at INTEGER,
    resolution TEXT
);

-- The allow-list invariant: the only thing the child API, search and the ranker read.
-- Playable = a live allow for (video, child or all children) in state auto-allowed/approved,
-- no block for (video, child or all children), and the video is known and available.
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
      AND p.removed_at IS NULL)
  AND NOT EXISTS (
    SELECT 1 FROM allowed_videos b
    WHERE b.youtube_id = v.youtube_id
      AND (b.child_id IS NULL OR b.child_id = c.id)
      AND b.state = 'blocked');

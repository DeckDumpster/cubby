-- The metadata worker's state: the day's YouTube quota spend by call, and each channel policy's
-- cached uploads playlist (so later runs skip the channels.list call).
CREATE TABLE yt_quota(
    day TEXT NOT NULL,  -- days since the epoch, UTC
    call TEXT NOT NULL,
    units INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY(day, call)
);

CREATE TABLE policy_cursor(
    policy_id TEXT PRIMARY KEY REFERENCES allow_policies(id),
    uploads_playlist TEXT NOT NULL
);

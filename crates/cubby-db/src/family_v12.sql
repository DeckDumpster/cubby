-- Playlists: a child's named lists of videos. `kind` is a closed set (only 'watch_later' today)
-- and each child has at most one playlist of each built-in kind, created on first use. An item
-- is only a pointer: what the child may see of it is still decided by the `playable` view.
CREATE TABLE playlists(
    id TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    kind TEXT NOT NULL CHECK (kind IN ('watch_later')),
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX playlists_builtin ON playlists(child_id, kind);

CREATE TABLE playlist_items(
    playlist_id TEXT NOT NULL REFERENCES playlists(id),
    youtube_id TEXT NOT NULL,
    added_at INTEGER NOT NULL,
    PRIMARY KEY (playlist_id, youtube_id)
);

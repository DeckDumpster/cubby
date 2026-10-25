-- Family-level settings (key/value), and when each child's limit was first seen spent today:
-- the start of the browse grace. The row is replaced once a day turns over or a grant lifts it.
CREATE TABLE family_settings(
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE limit_reached(
    child_id TEXT PRIMARY KEY REFERENCES children(id),
    reached_at INTEGER NOT NULL
);

-- A parent's extra allowance for today: it adds to the child's limits until expires_at, the
-- child's next local midnight, and is kept afterwards as the record of who granted what.
CREATE TABLE budget_grants(
    id TEXT PRIMARY KEY,
    child_id TEXT NOT NULL REFERENCES children(id),
    granted_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    minutes REAL NOT NULL DEFAULT 0,
    videos REAL NOT NULL DEFAULT 0,
    granted_by TEXT NOT NULL,
    note TEXT
);
CREATE INDEX budget_grants_child ON budget_grants(child_id, expires_at);

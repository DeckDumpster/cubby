-- Grants are a ledger: removing one never edits or deletes its budget_grants row, it appends a
-- reversal here naming who undid it and when. A reversed grant no longer counts toward the
-- child's limits; the history keeps showing it, marked removed.
CREATE TABLE budget_grant_reversals(
    grant_id TEXT PRIMARY KEY REFERENCES budget_grants(id),
    reversed_at INTEGER NOT NULL,
    reversed_by TEXT NOT NULL
);

-- A channel added by @handle is stored under its resolved UC id; this keeps the name the parent
-- typed for display until the channel's videos carry a title of their own.
CREATE TABLE policy_label(
    policy_id TEXT PRIMARY KEY REFERENCES allow_policies(id),
    label TEXT NOT NULL
);

-- The child's "Recently watched" read: one child's plays, newest first.
CREATE INDEX play_grants_child_granted ON play_grants(child_id, granted_at);

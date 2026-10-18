-- videos_fts is an external-content index (content='videos'): SQLite never fills it itself, so
-- these triggers mirror every insert, update and delete of a videos row, and the rebuild indexes
-- the rows an existing file already holds.
CREATE TRIGGER videos_fts_ai AFTER INSERT ON videos BEGIN
    INSERT INTO videos_fts(rowid, title, channel_title, tags)
    VALUES (new.rowid, new.title, new.channel_title, new.tags);
END;

CREATE TRIGGER videos_fts_ad AFTER DELETE ON videos BEGIN
    INSERT INTO videos_fts(videos_fts, rowid, title, channel_title, tags)
    VALUES ('delete', old.rowid, old.title, old.channel_title, old.tags);
END;

CREATE TRIGGER videos_fts_au AFTER UPDATE ON videos BEGIN
    INSERT INTO videos_fts(videos_fts, rowid, title, channel_title, tags)
    VALUES ('delete', old.rowid, old.title, old.channel_title, old.tags);
    INSERT INTO videos_fts(rowid, title, channel_title, tags)
    VALUES (new.rowid, new.title, new.channel_title, new.tags);
END;

INSERT INTO videos_fts(videos_fts) VALUES ('rebuild');

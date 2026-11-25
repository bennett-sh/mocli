CREATE TABLE entries (
    id            INTEGER PRIMARY KEY,
    kind          TEXT    NOT NULL,
    title         TEXT    NOT NULL,
    text          TEXT,
    hash          BLOB    NOT NULL UNIQUE,
    size_bytes    INTEGER NOT NULL,
    width         INTEGER,
    height        INTEGER,
    source_exe    TEXT,
    source_name   TEXT,
    source_title  TEXT,
    pinned        INTEGER NOT NULL DEFAULT 0,
    first_copied  INTEGER NOT NULL,
    last_copied   INTEGER NOT NULL,
    copy_count    INTEGER NOT NULL DEFAULT 1,
    last_pasted   INTEGER
);

CREATE INDEX entries_recent ON entries (pinned, last_copied);

-- Extra clipboard formats restored when pasting.
CREATE TABLE formats (
    entry_id  INTEGER NOT NULL REFERENCES entries (id) ON DELETE CASCADE,
    name      TEXT    NOT NULL,
    data      BLOB    NOT NULL,
    PRIMARY KEY (entry_id, name)
);

-- Trigrams give substring matching, which suits filtering as you type.
CREATE VIRTUAL TABLE entries_fts USING fts5 (
    title, text, source_name,
    content = 'entries', content_rowid = 'id', tokenize = 'trigram'
);

CREATE TRIGGER entries_inserted AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts (rowid, title, text, source_name)
    VALUES (new.id, new.title, new.text, new.source_name);
END;

CREATE TRIGGER entries_deleted AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts (entries_fts, rowid, title, text, source_name)
    VALUES ('delete', old.id, old.title, old.text, old.source_name);
END;

CREATE TRIGGER entries_updated AFTER UPDATE OF title, text, source_name ON entries BEGIN
    INSERT INTO entries_fts (entries_fts, rowid, title, text, source_name)
    VALUES ('delete', old.id, old.title, old.text, old.source_name);
    INSERT INTO entries_fts (rowid, title, text, source_name)
    VALUES (new.id, new.title, new.text, new.source_name);
END;

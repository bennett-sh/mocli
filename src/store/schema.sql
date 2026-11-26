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
    last_pasted   INTEGER,
    language      TEXT,
    link_title    TEXT,
    link_description TEXT,
    -- Text found in an image.
    ocr_text      TEXT,
    -- A name the user gave the entry, shown in place of its title. The content
    -- stays searchable, and so does the name.
    name          TEXT
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
    title, text, source_name, link_title, ocr_text, name,
    content = 'entries', content_rowid = 'id', tokenize = 'trigram'
);

CREATE TRIGGER entries_inserted AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts (rowid, title, text, source_name, link_title, ocr_text, name)
    VALUES (new.id, new.title, new.text, new.source_name, new.link_title, new.ocr_text,
            new.name);
END;

CREATE TRIGGER entries_deleted AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts
        (entries_fts, rowid, title, text, source_name, link_title, ocr_text, name)
    VALUES ('delete', old.id, old.title, old.text, old.source_name,
            old.link_title, old.ocr_text, old.name);
END;

CREATE TRIGGER entries_updated
AFTER UPDATE OF title, text, source_name, link_title, ocr_text, name ON entries BEGIN
    INSERT INTO entries_fts
        (entries_fts, rowid, title, text, source_name, link_title, ocr_text, name)
    VALUES ('delete', old.id, old.title, old.text, old.source_name,
            old.link_title, old.ocr_text, old.name);
    INSERT INTO entries_fts (rowid, title, text, source_name, link_title, ocr_text, name)
    VALUES (new.id, new.title, new.text, new.source_name, new.link_title, new.ocr_text,
            new.name);
END;

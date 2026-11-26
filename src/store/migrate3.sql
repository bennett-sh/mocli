-- A name the user gave an entry, shown in place of its title. The content
-- stays searchable, and so does the name.
ALTER TABLE entries ADD COLUMN name TEXT;

DROP TRIGGER entries_inserted;
DROP TRIGGER entries_deleted;
DROP TRIGGER entries_updated;
DROP TABLE entries_fts;

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

INSERT INTO entries_fts (entries_fts) VALUES ('rebuild');

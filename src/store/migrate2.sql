ALTER TABLE entries ADD COLUMN language TEXT;
ALTER TABLE entries ADD COLUMN link_title TEXT;
ALTER TABLE entries ADD COLUMN link_description TEXT;
ALTER TABLE entries ADD COLUMN ocr_text TEXT;

-- The search index gains link titles and text found in images.
DROP TRIGGER entries_inserted;
DROP TRIGGER entries_deleted;
DROP TRIGGER entries_updated;
DROP TABLE entries_fts;

CREATE VIRTUAL TABLE entries_fts USING fts5 (
    title, text, source_name, link_title, ocr_text,
    content = 'entries', content_rowid = 'id', tokenize = 'trigram'
);

CREATE TRIGGER entries_inserted AFTER INSERT ON entries BEGIN
    INSERT INTO entries_fts (rowid, title, text, source_name, link_title, ocr_text)
    VALUES (new.id, new.title, new.text, new.source_name, new.link_title, new.ocr_text);
END;

CREATE TRIGGER entries_deleted AFTER DELETE ON entries BEGIN
    INSERT INTO entries_fts
        (entries_fts, rowid, title, text, source_name, link_title, ocr_text)
    VALUES ('delete', old.id, old.title, old.text, old.source_name,
            old.link_title, old.ocr_text);
END;

CREATE TRIGGER entries_updated
AFTER UPDATE OF title, text, source_name, link_title, ocr_text ON entries BEGIN
    INSERT INTO entries_fts
        (entries_fts, rowid, title, text, source_name, link_title, ocr_text)
    VALUES ('delete', old.id, old.title, old.text, old.source_name,
            old.link_title, old.ocr_text);
    INSERT INTO entries_fts (rowid, title, text, source_name, link_title, ocr_text)
    VALUES (new.id, new.title, new.text, new.source_name, new.link_title, new.ocr_text);
END;

INSERT INTO entries_fts (entries_fts) VALUES ('rebuild');

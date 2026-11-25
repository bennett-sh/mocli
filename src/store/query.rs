//! Reading the history back: lists, search, previews and paste payloads.

use std::collections::HashSet;

use anyhow::Result;
use nucleo_matcher::{
    Config, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};
use rusqlite::{OptionalExtension as _, params, params_from_iter, types::Value};

use super::{Blob, Format, Kind, Store, hex};

/// Longest text handed to the preview.
const PREVIEW_CHARS: usize = 10_000;
const MAX_SIMILAR: usize = 10;
/// Fuzzy matches scoring less than this per typed character are too scattered
/// to be useful. Runs and word initials score above 20, letters picked from the
/// middle of words about 16 or less.
const MIN_SCORE_PER_CHAR: u32 = 19;

/// One row of the list.
#[derive(Clone, Debug)]
pub struct Summary {
    pub id: i64,
    pub kind: Kind,
    pub title: String,
    pub pinned: bool,
    pub last_copied: i64,
    /// A picture for the row, if the entry has one.
    pub thumbnail: Option<Blob>,
}

#[derive(Debug, Default)]
pub struct Results {
    /// Entries containing every word of the query, newest first.
    pub matches: Vec<Summary>,
    /// Entries whose title only matches loosely, best first.
    pub similar: Vec<Summary>,
}

/// Everything shown about the selected entry.
#[derive(Debug)]
pub struct Details {
    pub kind: Kind,
    pub pinned: bool,
    /// Cut off after a few thousand characters.
    pub text: Option<String>,
    pub characters: usize,
    pub words: usize,
    pub lines: usize,
    pub size_bytes: u64,
    pub dimensions: Option<(u32, u32)>,
    /// The highlighter's name for the language of code.
    pub language: Option<String>,
    pub source_exe: Option<String>,
    pub source_name: Option<String>,
    pub source_title: Option<String>,
    pub source_icon: Option<Blob>,
    pub last_copied: i64,
    pub copy_count: i64,
    /// The stored picture of an image or SVG entry.
    pub image: Option<Blob>,
    /// The icon of a single file.
    pub thumbnail: Option<Blob>,
    pub link: LinkDetails,
    /// Text recognized in an image.
    pub ocr_text: Option<String>,
}

/// What a link preview found, if one was fetched.
#[derive(Debug, Default)]
pub struct LinkDetails {
    pub title: Option<String>,
    pub description: Option<String>,
    pub image: Option<Blob>,
    pub icon: Option<Blob>,
}

/// What goes back on the clipboard.
#[derive(Debug)]
pub struct Payload {
    pub kind: Kind,
    pub text: Option<String>,
    pub image: Option<Blob>,
    pub formats: Vec<(Format, Vec<u8>)>,
}

fn existing(blob: Blob) -> Option<Blob> {
    blob.exists().then_some(blob)
}

/// A condition limiting results to `kinds`, with its values. `None` for all.
fn kind_condition(kinds: &[Kind]) -> Option<(String, Vec<Value>)> {
    if kinds.is_empty() {
        return None;
    }
    let placeholders = vec!["?"; kinds.len()].join(", ");
    let values = kinds
        .iter()
        .map(|kind| Value::from(kind.as_str().to_owned()))
        .collect();
    Some((format!("kind IN ({placeholders})"), values))
}

impl Store {
    /// Searches entries of the given kinds, or of every kind if none are given.
    pub fn search(&self, query: &str, kinds: &[Kind], limit: usize) -> Result<Results> {
        let kind_filter = kind_condition(kinds);
        let mut conditions: Vec<&str> = Vec::new();
        let mut values = Vec::new();
        if let Some((condition, names)) = &kind_filter {
            conditions.push(condition);
            values.extend(names.clone());
        }

        // The trigram index cannot find words shorter than three characters.
        let (long, short): (Vec<&str>, Vec<&str>) = query
            .split_whitespace()
            .partition(|word| word.chars().count() >= 3);
        if !long.is_empty() {
            conditions.push("id IN (SELECT rowid FROM entries_fts WHERE entries_fts MATCH ?)");
            let phrases: Vec<String> = long
                .iter()
                .map(|word| format!("\"{}\"", word.replace('"', "\"\"")))
                .collect();
            values.push(Value::from(phrases.join(" ")));
        }
        for word in &short {
            conditions.push(
                r"(title LIKE ? ESCAPE '\' OR text LIKE ? ESCAPE '\'
                   OR source_name LIKE ? ESCAPE '\' OR link_title LIKE ? ESCAPE '\'
                   OR ocr_text LIKE ? ESCAPE '\')",
            );
            let pattern = word
                .replace('\\', r"\\")
                .replace('%', r"\%")
                .replace('_', r"\_");
            values.extend(std::iter::repeat_n(Value::from(format!("%{pattern}%")), 5));
        }

        let matches = self.summaries(&conditions, values, limit)?;
        let similar = if long.is_empty() {
            Vec::new()
        } else {
            self.similar(query, kinds, &matches, limit)?
        };
        Ok(Results { matches, similar })
    }

    fn similar(
        &self,
        query: &str,
        kinds: &[Kind],
        exclude: &[Summary],
        limit: usize,
    ) -> Result<Vec<Summary>> {
        let kind_filter = kind_condition(kinds);
        let (conditions, values) = match &kind_filter {
            Some((condition, names)) => (vec![condition.as_str()], names.clone()),
            None => (Vec::new(), Vec::new()),
        };
        let excluded: HashSet<i64> = exclude.iter().map(|entry| entry.id).collect();

        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let typed = query.chars().filter(|c| !c.is_whitespace()).count() as u32;
        let mut matcher = Matcher::new(Config::DEFAULT);
        let mut buffer = Vec::new();
        let mut scored: Vec<(u32, Summary)> = self
            .summaries(&conditions, values, limit)?
            .into_iter()
            .filter(|entry| !excluded.contains(&entry.id))
            .filter_map(|entry| {
                let title = Utf32Str::new(&entry.title, &mut buffer);
                let score = pattern.score(title, &mut matcher)?;
                (score >= typed * MIN_SCORE_PER_CHAR).then_some((score, entry))
            })
            .collect();
        // Stable, so equal scores stay newest first.
        scored.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
        scored.truncate(MAX_SIMILAR);
        Ok(scored.into_iter().map(|(_, entry)| entry).collect())
    }

    /// Newest first.
    fn summaries(
        &self,
        conditions: &[&str],
        mut values: Vec<Value>,
        limit: usize,
    ) -> Result<Vec<Summary>> {
        let filter = if conditions.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conditions.join(" AND "))
        };
        values.push(Value::from(i64::try_from(limit).unwrap_or(i64::MAX)));

        let mut query = self.db.prepare_cached(&format!(
            "SELECT id, kind, title, pinned, last_copied, hash FROM entries {filter}
             ORDER BY last_copied DESC, id DESC LIMIT ?"
        ))?;
        let rows = query.query_map(params_from_iter(values), |row| {
            let Some(kind) = Kind::parse(&row.get::<_, String>(1)?) else {
                return Ok(None);
            };
            let hash = hex(row.get(5)?);
            Ok(Some(Summary {
                id: row.get(0)?,
                kind,
                title: row.get(2)?,
                pinned: row.get(3)?,
                last_copied: row.get(4)?,
                thumbnail: match kind {
                    Kind::Image => Some(self.blobs.thumbnail(&hash)),
                    Kind::Files => existing(self.blobs.thumbnail(&hash)),
                    Kind::Link => existing(self.blobs.favicon(&hash)),
                    Kind::Svg => existing(self.blobs.svg(&hash)),
                    Kind::Text | Kind::Email | Kind::Color | Kind::Code => None,
                },
            }))
        })?;
        Ok(rows
            .filter_map(Result::transpose)
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn details(&self, id: i64) -> Result<Option<Details>> {
        let details = self
            .db
            .query_row(
                "SELECT kind, text, size_bytes, width, height, source_name, source_title,
                        last_copied, copy_count, hash, language, source_exe,
                        link_title, link_description, pinned, ocr_text
                 FROM entries WHERE id = ?1",
                [id],
                |row| {
                    let Some(kind) = Kind::parse(&row.get::<_, String>(0)?) else {
                        return Ok(None);
                    };
                    let mut text: Option<String> = row.get(1)?;
                    let (mut characters, mut words, mut lines) = (0, 0, 0);
                    if let Some(text) = &mut text {
                        characters = text.chars().count();
                        words = text.split_whitespace().count();
                        lines = text.lines().count();
                        if let Some((end, _)) = text.char_indices().nth(PREVIEW_CHARS) {
                            text.truncate(end);
                            text.push('…');
                        }
                    }
                    let width: Option<u32> = row.get(3)?;
                    let height: Option<u32> = row.get(4)?;
                    let hash = hex(row.get(9)?);
                    let source_exe: Option<String> = row.get(11)?;
                    Ok(Some(Details {
                        kind,
                        pinned: row.get(14)?,
                        ocr_text: row.get(15)?,
                        text,
                        characters,
                        words,
                        lines,
                        size_bytes: row.get::<_, i64>(2)?.max(0) as u64,
                        dimensions: width.zip(height),
                        language: row.get(10)?,
                        source_name: row.get(5)?,
                        source_title: row.get(6)?,
                        source_icon: source_exe
                            .as_deref()
                            .and_then(|exe| existing(self.blobs.app_icon(exe))),
                        source_exe,
                        last_copied: row.get(7)?,
                        copy_count: row.get(8)?,
                        image: match kind {
                            Kind::Image => Some(self.blobs.image(&hash)),
                            Kind::Svg => existing(self.blobs.svg(&hash)),
                            _ => None,
                        },
                        thumbnail: (kind == Kind::Files)
                            .then(|| existing(self.blobs.thumbnail(&hash)))
                            .flatten(),
                        link: LinkDetails {
                            title: row.get(12)?,
                            description: row.get(13)?,
                            image: existing(self.blobs.link_image(&hash)),
                            icon: existing(self.blobs.favicon(&hash)),
                        },
                    }))
                },
            )
            .optional()?;
        Ok(details.flatten())
    }

    pub fn payload(&self, id: i64) -> Result<Option<Payload>> {
        let entry = self
            .db
            .query_row(
                "SELECT kind, text, hash FROM entries WHERE id = ?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, [u8; 32]>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((kind, text, hash)) = entry else {
            return Ok(None);
        };
        let Some(kind) = Kind::parse(&kind) else {
            return Ok(None);
        };

        let mut query = self
            .db
            .prepare_cached("SELECT name, data FROM formats WHERE entry_id = ?1 ORDER BY name")?;
        let formats = query
            .query_map([id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|(name, data)| Some((Format::parse(&name)?, data)))
            .collect();

        Ok(Some(Payload {
            kind,
            text,
            image: (kind == Kind::Image).then(|| self.blobs.image(&hex(hash))),
            formats,
        }))
    }

    /// Records that an entry was pasted or copied from the history. With
    /// `bump`, it also moves to the top of the list.
    pub fn mark_used(&mut self, id: i64, now: i64, bump: bool) -> Result<()> {
        self.db.execute(
            "UPDATE entries SET last_pasted = ?2,
                 last_copied = CASE WHEN ?3 THEN ?2 ELSE last_copied END
             WHERE id = ?1",
            params![id, now, bump],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::tests::{Scratch, picture, text};

    fn titles(entries: &[Summary]) -> Vec<&str> {
        entries.iter().map(|entry| entry.title.as_str()).collect()
    }

    fn filled(name: &str, contents: &[&str]) -> (Scratch, Store) {
        let scratch = Scratch::new(name);
        let mut store = Store::open(&scratch.0).unwrap();
        for (at, content) in contents.iter().enumerate() {
            store.record(&text(content, at as i64)).unwrap();
        }
        (scratch, store)
    }

    fn search(store: &Store, query: &str) -> Results {
        store.search(query, &[], 100).unwrap()
    }

    #[test]
    fn empty_query_lists_newest_first_pinned_or_not() {
        let (_scratch, store) = filled("list", &["one", "two", "three"]);
        store
            .db
            .execute("UPDATE entries SET pinned = 1 WHERE title = 'one'", [])
            .unwrap();
        let results = search(&store, "  ");
        assert_eq!(titles(&results.matches), ["three", "two", "one"]);
        assert!(results.similar.is_empty());
        assert_eq!(store.search("", &[], 2).unwrap().matches.len(), 2);
    }

    #[test]
    fn every_word_must_appear_in_any_order_and_case() {
        let (_scratch, store) = filled(
            "words",
            &[
                "cargo build --release",
                "Cargo test",
                "release notes",
                "Grüße aus Köln",
            ],
        );
        let found = |query| titles(&search(&store, query).matches).join("|");
        assert_eq!(found("release CARGO"), "cargo build --release");
        assert_eq!(found("ar"), "Cargo test|cargo build --release");
        // Short and long words combine.
        assert_eq!(found("no release"), "release notes");
        assert_eq!(found("GRÜSSE"), "");
        assert_eq!(found("üße kö"), "Grüße aus Köln");
    }

    #[test]
    fn punctuation_is_searched_literally() {
        let (_scratch, store) = filled(
            "literal",
            &[
                r#"say "hi" now"#,
                "100% done",
                "snake_case",
                r"C:\temp",
                "plain",
            ],
        );
        for (query, expected) in [
            (r#""hi""#, r#"say "hi" now"#),
            ("%", "100% done"),
            ("_c", "snake_case"),
            (r"\t", r"C:\temp"),
            ("AND OR NOT *", ""),
        ] {
            assert_eq!(
                titles(&search(&store, query).matches).join("|"),
                expected,
                "{query:?}"
            );
        }
    }

    #[test]
    fn loose_matches_are_listed_separately() {
        let (_scratch, store) = filled(
            "fuzzy",
            &[
                "https://github.com/longbridge/gpui-component",
                "cargo build --release",
                "a big green house under blue skies",
            ],
        );
        let results = search(&store, "gthub");
        assert!(results.matches.is_empty());
        assert_eq!(
            titles(&results.similar),
            ["https://github.com/longbridge/gpui-component"]
        );
        assert_eq!(
            titles(&search(&store, "cargobuild").similar),
            ["cargo build --release"]
        );
        // Letters picked from the middle of words are not a match.
        assert!(search(&store, "igusls").similar.is_empty());
        // An exact match is never repeated as a loose one.
        assert!(search(&store, "github").similar.is_empty());
    }

    #[test]
    fn kind_filter_applies_to_all_results() {
        let (_scratch, mut store) = filled("kinds", &["a picture of text"]);
        store.record(&picture("Image (1×1)", 9)).unwrap();
        let images = store.search("", &[Kind::Image, Kind::Svg], 10).unwrap();
        assert_eq!(titles(&images.matches), ["Image (1×1)"]);
        assert!(images.matches[0].thumbnail.as_ref().unwrap().exists());
        let filtered = store.search("pictre", &[Kind::Image], 10).unwrap();
        assert!(filtered.matches.is_empty() && filtered.similar.is_empty());
    }

    #[test]
    fn details_and_payload_describe_the_entry() {
        let scratch = Scratch::new("details");
        let mut store = Store::open(&scratch.0).unwrap();
        let mut long = text(&"word\n".repeat(3000), 1);
        long.formats = vec![(Format::Rtf, vec![2]), (Format::Html, vec![1])];
        let id = store.record(&long).unwrap().id;

        let details = store.details(id).unwrap().unwrap();
        assert_eq!(
            (details.characters, details.words, details.lines),
            (15_000, 3000, 3000)
        );
        assert_eq!(details.text.unwrap().chars().count(), PREVIEW_CHARS + 1);
        assert!(details.source_icon.is_none() && details.link.title.is_none());

        let payload = store.payload(id).unwrap().unwrap();
        assert_eq!(payload.text.unwrap().len(), 15_000);
        assert_eq!(
            payload.formats,
            [(Format::Html, vec![1]), (Format::Rtf, vec![2])]
        );
        assert!(store.details(999).unwrap().is_none());
        assert!(store.payload(999).unwrap().is_none());
    }

    #[test]
    fn reusing_can_move_an_entry_to_the_top() {
        let (_scratch, mut store) = filled("used", &["old", "new"]);
        let old = search(&store, "old").matches[0].id;
        store.mark_used(old, 50, false).unwrap();
        assert_eq!(titles(&search(&store, "").matches), ["new", "old"]);
        store.mark_used(old, 60, true).unwrap();
        assert_eq!(titles(&search(&store, "").matches), ["old", "new"]);
    }
}

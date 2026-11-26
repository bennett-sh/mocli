//! Guesses whether copied text is source code, and in which language.
//!
//! The bar is deliberately high: text shown as code when it is prose looks
//! worse than code shown as text.

/// A language, by the name the highlighter knows and the one people read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    pub token: &'static str,
    pub name: &'static str,
}

macro_rules! languages {
    ($($constant:ident = $token:literal, $name:literal;)*) => {
        $(pub const $constant: Language = Language { token: $token, name: $name };)*
        const ALL: &[Language] = &[$($constant),*];
    };
}

languages! {
    BASH = "sh", "Shell";
    C = "c", "C";
    CPP = "cpp", "C++";
    CSHARP = "cs", "C#";
    CSS = "css", "CSS";
    GO = "go", "Go";
    HTML = "html", "HTML";
    JAVA = "java", "Java";
    JAVASCRIPT = "js", "JavaScript";
    JSON = "json", "JSON";
    POWERSHELL = "ps1", "PowerShell";
    PYTHON = "py", "Python";
    RUST = "rs", "Rust";
    SQL = "sql", "SQL";
    TOML = "toml", "TOML";
    TYPESCRIPT = "ts", "TypeScript";
    XML = "xml", "XML";
    YAML = "yaml", "YAML";
}

pub fn by_token(token: &str) -> Option<Language> {
    ALL.iter().find(|language| language.token == token).copied()
}

/// Only the start of very long text is examined.
const SAMPLE_BYTES: usize = 20_000;

pub fn detect(text: &str) -> Option<Language> {
    let text = text.trim();
    if text.len() < 8 {
        return None;
    }
    if is_json(text) {
        return Some(JSON);
    }
    let mut end = text.len().min(SAMPLE_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let sample = &text[..end];
    let lines: Vec<&str> = sample
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect();

    markup(sample)
        .or_else(|| shebang(&lines))
        .or_else(|| sql(sample))
        .or_else(|| by_signals(sample, &lines))
        .or_else(|| structured(&lines))
}

fn is_json(text: &str) -> bool {
    (text.starts_with('{') || text.starts_with('['))
        && serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| match value {
            serde_json::Value::Object(map) => !map.is_empty(),
            // `[1]` is more likely a footnote than a document.
            serde_json::Value::Array(items) => items.len() > 1,
            _ => false,
        })
}

fn markup(text: &str) -> Option<Language> {
    if !text.starts_with('<') || !text.ends_with('>') {
        return None;
    }
    if text.starts_with("<?xml") {
        return Some(XML);
    }
    let lower = text.to_ascii_lowercase();
    (lower.starts_with("<!doctype html") || text.contains("</") || text.contains("/>"))
        .then_some(HTML)
}

fn shebang(lines: &[&str]) -> Option<Language> {
    let first = lines.first()?.strip_prefix("#!")?;
    if first.contains("python") {
        Some(PYTHON)
    } else if first.contains("node") {
        Some(JAVASCRIPT)
    } else if first.contains("pwsh") {
        Some(POWERSHELL)
    } else {
        first.contains("sh").then_some(BASH)
    }
}

fn sql(text: &str) -> Option<Language> {
    // Keywords written in capitals or at the very start of a statement.
    let upper = text.to_ascii_uppercase();
    let starts = |keyword: &str| upper.starts_with(keyword);
    let has = |keyword: &str| {
        upper
            .split(|c: char| !c.is_ascii_alphabetic())
            .any(|word| word == keyword)
    };
    let statement = (starts("SELECT ") || starts("SELECT\n")) && has("FROM")
        || starts("INSERT INTO ")
        || starts("UPDATE ") && has("SET") && text.contains('=')
        || starts("DELETE FROM ")
        || starts("CREATE TABLE ")
        || starts("ALTER TABLE ");
    // "Select a file from the list" is English.
    let shouted = text.starts_with(|c: char| c.is_ascii_uppercase())
        && text.chars().nth(1).is_some_and(|c| c.is_ascii_uppercase());
    let symbols = text.contains(['*', ';', '=', '(', ',', '_']);
    (statement && (shouted || symbols)).then_some(SQL)
}

/// Fragments that are typical of one language and rare in prose.
const SIGNALS: &[(Language, &[&str])] = &[
    (
        RUST,
        &[
            "fn ",
            "let mut ",
            "impl ",
            "pub fn ",
            "pub struct ",
            "use std::",
            "-> ",
            "println!(",
            "#[derive",
            "#[cfg",
            "&self",
            "Some(",
            "=> {",
            "match ",
            ".unwrap()",
            "::new(",
        ],
    ),
    (
        TYPESCRIPT,
        &[
            "interface ",
            ": string",
            ": number",
            ": boolean",
            "export type ",
            "readonly ",
            "<string>",
            " as const",
        ],
    ),
    (
        JAVASCRIPT,
        &[
            "const ",
            "=> ",
            "function ",
            "console.log(",
            "===",
            "!==",
            "require(",
            "document.",
            "await ",
            "export default",
            "import {",
            "} from ",
        ],
    ),
    (
        PYTHON,
        &[
            "def ", "elif ", "self.", "__name__", "print(", "):\n", "import ", "from ", "None",
            "lambda ",
        ],
    ),
    (
        GO,
        &[
            "package ",
            "func ",
            ":= ",
            "fmt.",
            "err != nil",
            "chan ",
            "go func",
        ],
    ),
    (
        CSHARP,
        &[
            "using System",
            "namespace ",
            "public class ",
            "Console.Write",
            "var ",
            "async Task",
            "{ get; set; }",
        ],
    ),
    (
        JAVA,
        &[
            "public static void ",
            "System.out.print",
            "import java.",
            "public class ",
            "@Override",
            "private final ",
        ],
    ),
    (
        CPP,
        &[
            "#include <",
            "std::",
            "template<",
            "cout <<",
            "nullptr",
            "int main(",
        ],
    ),
    (
        C,
        &[
            "#include <",
            "int main(",
            "printf(",
            "sizeof(",
            "#define ",
            "->",
        ],
    ),
    (
        POWERSHELL,
        &[
            "$_",
            "-ErrorAction",
            "Get-",
            "Set-",
            "Write-Host",
            "$env:",
            "Where-Object",
            "ForEach-Object",
            "param(",
            " -eq ",
            "New-Object",
        ],
    ),
    (
        BASH,
        &[
            "sudo ", "apt-get ", "export ", "echo ", "| grep ", "&& ", "chmod ", "$(", "fi\n",
            "done\n", "--no-",
        ],
    ),
];

fn by_signals(text: &str, lines: &[&str]) -> Option<Language> {
    // Lines that end or begin the way code does.
    let codelike = lines
        .iter()
        .filter(|line| {
            line.ends_with([';', '{', '}', '(', ')', ',', ':', '|', '\\', '`'])
                || line.starts_with("    ")
                || line.starts_with('\t')
                || line
                    .trim_start()
                    .starts_with(['}', ')', '#', '$', '@', '/'])
        })
        .count();
    let shaped = codelike * 2 >= lines.len();

    let (language, score) = SIGNALS
        .iter()
        .map(|(language, signals)| {
            let score = signals.iter().filter(|s| text.contains(**s)).count();
            (*language, score)
        })
        // The first of equals wins, so order the table by preference.
        .rev()
        .max_by_key(|(_, score)| *score)?;

    // Single commands such as `cargo build` stay text: they read fine as is.
    let enough = if shaped { 2 } else { 4 };
    if score < enough {
        return None;
    }
    // TypeScript is JavaScript with annotations.
    if language == JAVASCRIPT && text.contains(": string") {
        return Some(TYPESCRIPT);
    }
    Some(language)
}

/// Configuration formats and style sheets, recognized by their line shapes.
fn structured(lines: &[&str]) -> Option<Language> {
    if lines.len() < 2 {
        return None;
    }
    let count = |test: fn(&str) -> bool| lines.iter().filter(|line| test(line)).count();
    let key = |name: &str| {
        !name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "_-.\"".contains(c))
    };

    let declarations = count(|line| {
        let line = line.trim();
        line.ends_with(';') && line.contains(": ")
    });
    let blocks = count(|line| line.ends_with('{'));
    if blocks > 0 && declarations > 0 && lines.last()?.ends_with('}') {
        return Some(CSS);
    }

    let tables = count(|line| line.starts_with('[') && line.ends_with(']'));
    let assignments = lines
        .iter()
        .filter(|line| line.split_once(" = ").is_some_and(|(name, _)| key(name)))
        .count();
    if tables > 0 && assignments > 0 && (tables + assignments) * 4 >= lines.len() * 3 {
        return Some(TOML);
    }

    let entries = lines
        .iter()
        .filter(|line| {
            let line = line.trim_start();
            let line = line.strip_prefix("- ").unwrap_or(line);
            match line.split_once(':') {
                Some((name, rest)) => key(name) && (rest.is_empty() || rest.starts_with(' ')),
                None => false,
            }
        })
        .count();
    let items = count(|line| line.trim_start().starts_with("- "));
    let nested = count(|line| line.starts_with("  "));
    (lines.len() >= 3 && nested > 0 && entries > 1 && (entries + items) >= lines.len())
        .then_some(YAML)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_common_languages() {
        for (text, expected) in [
            (r#"{"name": "mocli", "version": 1}"#, JSON),
            ("<div class=\"a\">\n  <p>hi</p>\n</div>", HTML),
            ("<?xml version=\"1.0\"?><a/>", XML),
            ("#!/usr/bin/env python3\nprint(1)", PYTHON),
            ("SELECT id, title FROM entries WHERE pinned = 1;", SQL),
            ("fn main() {\n    println!(\"hi\");\n}", RUST),
            (
                "const items = await load();\nitems.map((x) => console.log(x));",
                JAVASCRIPT,
            ),
            (
                "interface User {\n  name: string;\n  age: number;\n}",
                TYPESCRIPT,
            ),
            (
                "def area(self):\n    return self.w * self.h\n\nprint(area(None))",
                PYTHON,
            ),
            (
                "package main\n\nfunc main() {\n    x := 1\n    fmt.Println(x)\n}",
                GO,
            ),
            (
                "using System;\nnamespace App {\n    public class A { }\n}",
                CSHARP,
            ),
            (
                "#include <stdio.h>\nint main(void) {\n    printf(\"x\");\n}",
                C,
            ),
            (
                "Get-ChildItem $env:TEMP |\n    Where-Object { $_.Length -eq 0 }",
                POWERSHELL,
            ),
            ("body {\n  color: red;\n  margin: 0 auto;\n}", CSS),
            ("[package]\nname = \"mocli\"\nversion = \"1.0\"", TOML),
            ("name: build\non:\n  push:\n    branches: [main]", YAML),
        ] {
            assert_eq!(detect(text), Some(expected), "{text:?}");
        }
    }

    #[test]
    fn leaves_prose_and_commands_alone() {
        for text in [
            "hi",
            "The quick brown fox jumps over the lazy dog.",
            "Select a file from the list and press Enter.",
            "Note: the meeting moved.\nAgenda: budget, hiring.",
            "Dear Sam,\n\nthanks for the notes. I'll read them from home;\nmore soon.\n\nBest",
            "cargo build --release",
            "Stop-Process -Name explorer -Force",
            "1. first\n2. second\n3. third",
            "[1]",
            "<3 this",
            "Shopping:\n- milk\n- eggs",
        ] {
            assert_eq!(detect(text), None, "{text:?}");
        }
    }

    #[test]
    fn long_multibyte_text_does_not_split_a_character() {
        assert_eq!(detect(&"é".repeat(SAMPLE_BYTES)), None);
    }

    #[test]
    fn tokens_round_trip() {
        for language in ALL {
            assert_eq!(by_token(language.token), Some(*language));
        }
    }
}

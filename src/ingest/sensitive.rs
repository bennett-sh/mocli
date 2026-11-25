//! Recognizes secrets that should not be written to disk.
//!
//! Only unmistakable shapes are matched: skipping an ordinary copy would be
//! worse than missing an unusual secret, which apps can mark as private.

/// Token formats that services document, as prefix and minimum total length.
const TOKEN_PREFIXES: &[(&str, usize)] = &[
    ("ghp_", 40),
    ("gho_", 40),
    ("ghu_", 40),
    ("ghs_", 40),
    ("ghr_", 40),
    ("github_pat_", 60),
    ("glpat-", 26),
    ("sk-ant-", 40),
    ("sk-proj-", 40),
    ("sk_live_", 30),
    ("rk_live_", 30),
    ("xoxb-", 40),
    ("xoxp-", 40),
    ("AKIA", 20),
    ("ASIA", 20),
    ("AIza", 39),
    ("npm_", 40),
    ("pypi-", 50),
    ("SG.", 60),
];

pub fn is_sensitive(text: &str) -> bool {
    let text = text.trim();
    is_private_key(text) || is_card_number(text) || text.split_whitespace().any(is_token)
}

fn is_private_key(text: &str) -> bool {
    text.contains("-----BEGIN") && text.contains("PRIVATE KEY")
}

fn is_token(word: &str) -> bool {
    // Tokens often arrive quoted or as `NAME=value`.
    let word = word
        .rsplit(['=', ':', '"', '\'', '`'])
        .find(|part| !part.is_empty() && !matches!(*part, ";" | ","))
        .unwrap_or_default()
        .trim_end_matches([';', ',']);
    let valid = |c: char| c.is_ascii_alphanumeric() || "-_.".contains(c);
    let known = TOKEN_PREFIXES.iter().any(|(prefix, length)| {
        word.len() >= *length && word.starts_with(prefix) && word.chars().all(valid)
    });
    // A signed web token: three encoded parts, the first two JSON objects.
    let mut parts = word.split('.');
    let web_token = match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(header), Some(body), Some(signature), None) => {
            header.starts_with("eyJ")
                && body.starts_with("eyJ")
                && signature.len() >= 16
                && word.chars().all(valid)
        }
        _ => false,
    };
    known || web_token
}

/// A payment card number on its own: the right length, grouping and check digit.
fn is_card_number(text: &str) -> bool {
    if !text
        .chars()
        .all(|c| c.is_ascii_digit() || c == ' ' || c == '-')
    {
        return false;
    }
    let digits: Vec<u32> = text.chars().filter_map(|c| c.to_digit(10)).collect();
    // The major networks start with 2 to 6; this excludes most other numbers.
    if !(13..=19).contains(&digits.len()) || !(2..=6).contains(&digits[0]) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(position, &digit)| match position % 2 {
            0 => digit,
            _ if digit > 4 => digit * 2 - 9,
            _ => digit * 2,
        })
        .sum();
    sum.is_multiple_of(10)
}

#[cfg(test)]
mod tests {
    // Not `super::*`: GPUI exports a `test` attribute of its own.
    use super::is_sensitive;

    /// Built at run time, so secret scanners do not flag the source.
    fn token(prefix: &str, length: usize) -> String {
        let filler: String = "a1B2".chars().cycle().take(length - prefix.len()).collect();
        format!("{prefix}{filler}")
    }

    #[test]
    fn recognizes_secrets() {
        let web = format!("eyJhbGciOiJ9.eyJzdWIiOiJ9.{}", "s".repeat(20));
        for text in [
            token("ghp_", 40),
            format!("export TOKEN={}", token("github_pat_", 70)),
            format!("key: \"{}\";", token("sk_live_", 32)),
            token("AKIA", 20),
            web,
            "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n".to_owned(),
            "4111 1111 1111 1111".to_owned(),
            "5500-0000-0000-0004".to_owned(),
        ] {
            assert!(is_sensitive(&text), "{text:?} passed");
        }
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        for text in [
            "",
            "ghp_short",
            "the prefix ghp_ is used by GitHub",
            "4111 1111 1111 1112",
            "1234567812345670",
            "order 4111111111111111 shipped",
            "+49 30 1234567",
            "https://example.com/AKIA",
            "eyJ.eyJ.x",
            "-----BEGIN CERTIFICATE-----",
        ] {
            assert!(!is_sensitive(text), "{text:?} was held back");
        }
    }
}

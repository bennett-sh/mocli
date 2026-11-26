//! Link previews: a page's title, description, picture and icon.
//!
//! Fetching a copied address tells its server about it and can use up one-time
//! links, so this is off unless switched on, and addresses that look private or
//! carry secrets are never requested.

use std::{io::Read as _, time::Duration};

use anyhow::{Context as _, Result};
use url::{Host, Url};

use super::picture;

const TIMEOUT: Duration = Duration::from_secs(8);
const MAX_REDIRECTS: usize = 5;
const MAX_PAGE_BYTES: u64 = 512 * 1024;
const MAX_IMAGE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_IMAGE_EDGE: u32 = 800;
const ICON_EDGE: u32 = 64;
const MAX_TEXT_CHARS: usize = 300;

#[derive(Debug, Default)]
pub struct Preview {
    pub title: Option<String>,
    pub description: Option<String>,
    /// PNG data.
    pub image: Option<Vec<u8>>,
    /// PNG data.
    pub icon: Option<Vec<u8>>,
}

/// Whether a copied address may be requested at all.
pub fn is_fetchable(address: &str) -> bool {
    parse(address).is_some_and(|url| is_public(&url) && !carries_secret(&url))
}

fn parse(address: &str) -> Option<Url> {
    if address.starts_with("www.") {
        Url::parse(&format!("https://{address}")).ok()
    } else {
        Url::parse(address).ok()
    }
}

/// A web address on a public host name, without credentials.
fn is_public(url: &Url) -> bool {
    let named = match url.host() {
        Some(Host::Domain(name)) => {
            let name = name.trim_end_matches('.').to_ascii_lowercase();
            name.contains('.')
                && ![
                    ".local",
                    ".localhost",
                    ".internal",
                    ".lan",
                    ".home",
                    ".test",
                    ".arpa",
                ]
                .iter()
                .any(|suffix| name.ends_with(suffix))
        }
        // Bare addresses are mostly routers and other machines nearby.
        _ => false,
    };
    named
        && matches!(url.scheme(), "http" | "https")
        && url.username().is_empty()
        && url.password().is_none()
}

/// Password resets, sign-in links and signed downloads.
fn carries_secret(url: &Url) -> bool {
    const NAMES: &[&str] = &[
        "token", "key", "secret", "password", "passwd", "pwd", "auth", "session", "code", "sig",
        "otp", "magic", "ticket", "invite", "reset", "confirm", "verify",
    ];
    let named = |text: &str| {
        let text = text.to_ascii_lowercase();
        NAMES.iter().any(|name| text.contains(name))
    };
    // A long run of mixed letters and digits is probably a credential.
    let random = |text: &str| {
        text.len() >= 24
            && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && text.chars().any(|c| c.is_ascii_digit())
            && text.chars().any(|c| c.is_ascii_alphabetic())
    };
    url.query_pairs()
        .any(|(name, value)| named(&name) || random(&value))
        || url
            .path_segments()
            .is_some_and(|mut segments| segments.any(|s| random(s) || s.len() > 80))
        || url.fragment().is_some_and(|f| named(f) && f.contains('='))
}

pub fn fetch(address: &str) -> Result<Preview> {
    let url = parse(address).context("not an address")?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(TIMEOUT))
        // Followed by hand, to check every destination.
        .max_redirects(0)
        .max_redirects_will_error(false)
        .user_agent("mocli link preview")
        .build()
        .into();

    let (page, base, kind) = get(&agent, url, MAX_PAGE_BYTES, false)?;
    anyhow::ensure!(
        kind.starts_with("text/html") || kind.starts_with("application/xhtml"),
        "not a web page"
    );
    let meta = parse_page(&String::from_utf8_lossy(&page));

    let download = |reference: &str, edge| -> Option<Vec<u8>> {
        let url = base.join(reference).ok()?;
        let (data, _, _) = get(&agent, url, MAX_IMAGE_BYTES, true).ok()?;
        picture::shrink(&data, edge).ok()
    };
    Ok(Preview {
        title: meta.title,
        description: meta.description,
        image: meta
            .image
            .and_then(|reference| download(&reference, MAX_IMAGE_EDGE)),
        icon: download(meta.icon.as_deref().unwrap_or("/favicon.ico"), ICON_EDGE),
    })
}

/// Returns the body, the final address and the content type. With `complete`,
/// a body over the limit is an error instead of being cut off.
fn get(
    agent: &ureq::Agent,
    mut url: Url,
    limit: u64,
    complete: bool,
) -> Result<(Vec<u8>, Url, String)> {
    for _ in 0..=MAX_REDIRECTS {
        anyhow::ensure!(is_public(&url), "refusing to request a private address");
        let mut response = agent.get(url.as_str()).call()?;
        let header = |name: &str| {
            response
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned()
        };
        if response.status().is_redirection() {
            url = url.join(&header("location"))?;
            continue;
        }
        anyhow::ensure!(response.status().is_success(), "{}", response.status());
        let kind = header("content-type").to_ascii_lowercase();
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .take(limit + 1)
            .read_to_end(&mut body)?;
        if body.len() as u64 > limit {
            anyhow::ensure!(!complete, "too large");
            body.truncate(limit as usize);
        }
        return Ok((body, url, kind));
    }
    anyhow::bail!("too many redirects")
}

#[derive(Debug, Default, PartialEq)]
struct Meta {
    title: Option<String>,
    description: Option<String>,
    image: Option<String>,
    icon: Option<String>,
}

/// Reads the few tags a preview needs without parsing the whole document.
fn parse_page(html: &str) -> Meta {
    let mut meta = Meta::default();
    let mut page_title = None;
    let mut page_description = None;
    // ASCII lowercasing keeps byte positions intact.
    let lower = html.to_ascii_lowercase();
    for (start, _) in lower.match_indices('<') {
        let rest = &lower[start + 1..];
        let name_length = rest
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(rest.len());
        let body = &html[start + 1 + name_length..];
        match &rest[..name_length] {
            "title" if page_title.is_none() => {
                if let Some(open) = body.find('>') {
                    let inner = &body[open + 1..];
                    let end = inner.find('<').unwrap_or(inner.len());
                    page_title = clean(&inner[..end]);
                }
            }
            "meta" => {
                let attributes = attributes(body);
                let get = |key| lookup(&attributes, key);
                let Some(content) = get("content") else {
                    continue;
                };
                match get("property").or_else(|| get("name")) {
                    Some("og:title") => meta.title = clean(content),
                    Some("og:description") => meta.description = clean(content),
                    Some("description") => page_description = clean(content),
                    Some("og:image" | "twitter:image") if meta.image.is_none() => {
                        meta.image = clean_address(content);
                    }
                    _ => {}
                }
            }
            "link" if meta.icon.is_none() => {
                let attributes = attributes(body);
                let relation = lookup(&attributes, "rel").unwrap_or_default();
                if relation
                    .split_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("icon"))
                {
                    meta.icon = lookup(&attributes, "href")
                        .and_then(clean_address)
                        // Vector icons cannot be decoded here.
                        .filter(|href| !href.ends_with(".svg"));
                }
            }
            _ => {}
        }
    }
    meta.title = meta.title.or(page_title);
    meta.description = meta.description.or(page_description);
    meta
}

fn lookup<'a>(attributes: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.as_str())
}

/// The attributes of a tag, given the text following its name.
fn attributes(mut text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    loop {
        text = text.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if text.is_empty() || text.starts_with('>') || found.len() == 32 {
            return found;
        }
        let end = text
            .find(|c: char| c.is_whitespace() || matches!(c, '=' | '>' | '/'))
            .unwrap_or(text.len());
        if end == 0 {
            return found;
        }
        let name = text[..end].to_ascii_lowercase();
        text = text[end..].trim_start();
        let Some(rest) = text.strip_prefix('=') else {
            found.push((name, String::new()));
            continue;
        };
        let rest = rest.trim_start();
        let (value, after) = match rest.chars().next() {
            Some(quote @ ('"' | '\'')) => {
                let inner = &rest[1..];
                let end = inner.find(quote).unwrap_or(inner.len());
                (&inner[..end], &inner[(end + 1).min(inner.len())..])
            }
            _ => {
                let end = rest
                    .find(|c: char| c.is_whitespace() || c == '>')
                    .unwrap_or(rest.len());
                rest.split_at(end)
            }
        };
        found.push((name, value.to_owned()));
        text = after;
    }
}

fn clean(text: &str) -> Option<String> {
    let joined = decode_entities(text)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cut: String = joined.chars().take(MAX_TEXT_CHARS).collect();
    (!cut.is_empty()).then_some(cut)
}

fn clean_address(text: &str) -> Option<String> {
    let address = decode_entities(text.trim());
    (!address.is_empty() && !address.starts_with("data:")).then_some(address)
}

fn decode_entities(text: &str) -> String {
    let mut decoded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        decoded.push_str(&rest[..start]);
        rest = &rest[start..];
        let entity = rest[1..]
            .split_once(';')
            .filter(|(name, _)| name.len() <= 10)
            .and_then(|(name, after)| {
                let character = match name {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    "nbsp" => Some(' '),
                    "ndash" => Some('–'),
                    "mdash" => Some('—'),
                    _ => {
                        let number = name.strip_prefix('#')?;
                        let code = match number.strip_prefix(['x', 'X']) {
                            Some(hex) => u32::from_str_radix(hex, 16).ok()?,
                            None => number.parse().ok()?,
                        };
                        char::from_u32(code)
                    }
                }?;
                Some((character, after))
            });
        match entity {
            Some((character, after)) => {
                decoded.push(character);
                rest = after;
            }
            None => {
                decoded.push('&');
                rest = &rest[1..];
            }
        }
    }
    decoded.push_str(rest);
    decoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_only_harmless_public_addresses() {
        for address in [
            "https://github.com/bennett-sh/mocli",
            "http://example.com/a/long-readable-article-title-about-things?page=2",
            "www.example.com",
        ] {
            assert!(is_fetchable(address), "{address}");
        }
        for address in [
            "ftp://example.com",
            "https://localhost:3000",
            "http://192.168.0.1/",
            "http://[::1]/",
            "http://router.lan/",
            "http://intranet/",
            "https://user:pass@example.com",
            "https://example.com/reset?token=abc",
            "https://example.com/login?sessionId=1",
            "https://example.com/d/8f3a9c1b2e7d4f6a9b0c1d2e3f4a5b6c",
            "https://example.com/?x=8f3a9c1b2e7d4f6a9b0c1d2e3f4a5b6c",
            "https://example.com/cb#access_token=abc",
        ] {
            assert!(!is_fetchable(address), "{address}");
        }
    }

    #[test]
    fn prefers_open_graph_tags() {
        let page = r#"<html><head>
            <TITLE>Fallback</TITLE>
            <meta name="description" content="plain">
            <meta property="og:title" content="Tom &amp; Jerry&#39;s &#x2014; page">
            <meta content='/cover.png' property='og:image' />
            <link rel="shortcut icon" href=/fav.png>
        </head></html>"#;
        assert_eq!(
            parse_page(page),
            Meta {
                title: Some("Tom & Jerry's — page".into()),
                description: Some("plain".into()),
                image: Some("/cover.png".into()),
                icon: Some("/fav.png".into()),
            }
        );
    }

    #[test]
    fn survives_broken_markup() {
        for page in [
            "",
            "<",
            "<meta",
            "<meta content=\"unterminated",
            "<title>é",
            "<link rel=icon href=",
            "<meta = = =>",
            "&&#;&#xZZ;&é;",
        ] {
            parse_page(page);
            decode_entities(page);
        }
        assert_eq!(parse_page("<title>é").title.as_deref(), Some("é"));
        assert_eq!(decode_entities("a & b &unknown; c"), "a & b &unknown; c");
    }
}

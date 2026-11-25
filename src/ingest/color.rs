//! Recognizes text that is exactly one CSS color.
//!
//! Named colors are left out: "red", "tan" or "gold" are usually just words.

pub fn is_color(text: &str) -> bool {
    let text = text.to_ascii_lowercase();
    if let Some(digits) = text.strip_prefix('#') {
        return matches!(digits.len(), 3 | 4 | 6 | 8)
            && digits.bytes().all(|b| b.is_ascii_hexdigit());
    }

    let Some((function, rest)) = text.split_once('(') else {
        return false;
    };
    let Some(arguments) = rest.strip_suffix(')') else {
        return false;
    };
    if !matches!(function, "rgb" | "rgba" | "hsl" | "hsla" | "oklch") {
        return false;
    }

    // Both `rgb(1, 2, 3)` and `rgb(1 2 3 / 50%)`.
    let parts: Vec<&str> = arguments
        .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
        .filter(|part| !part.is_empty())
        .collect();
    matches!(parts.len(), 3 | 4) && parts.into_iter().all(is_component)
}

fn is_component(part: &str) -> bool {
    let number = ["%", "deg", "turn", "rad"]
        .iter()
        .find_map(|unit| part.strip_suffix(unit))
        .unwrap_or(part);
    part == "none" || number.parse::<f32>().is_ok_and(f32::is_finite)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_css_colors() {
        for text in [
            "#fff",
            "#FF7A1A",
            "#ff7a1a80",
            "rgb(255, 122, 26)",
            "rgba(255,122,26,0.5)",
            "rgb(255 122 26 / 50%)",
            "hsl(25deg 100% 55%)",
            "oklch(0.72 0.18 50)",
        ] {
            assert!(is_color(text), "{text:?} was rejected");
        }
    }

    #[test]
    fn rejects_everything_else() {
        for text in [
            "",
            "#",
            "#ff",
            "#fffff",
            "#ggg",
            "ff7a1a",
            "red",
            "rgb(1, 2)",
            "rgb(a, b, c)",
            "rgb(1, 2, 3) and more",
            "print(1, 2, 3)",
            "# heading",
        ] {
            assert!(!is_color(text), "{text:?} was accepted");
        }
    }
}

//! CSS colors: recognizing them, converting between notations and judging
//! contrast.
//!
//! Named colors are left out: "red", "tan" or "gold" are usually just words.

use std::f32::consts::PI;

/// An sRGB color. Channels are 0 to 255, alpha is 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
    pub alpha: f32,
}

impl Color {
    /// Parses text that is exactly one color: hex, `rgb()`, `hsl()` or `oklch()`.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim().to_ascii_lowercase();
        if let Some(digits) = text.strip_prefix('#') {
            return parse_hex(digits);
        }

        let (function, rest) = text.split_once('(')?;
        let arguments = rest.strip_suffix(')')?;
        // Both `rgb(1, 2, 3)` and `rgb(1 2 3 / 50%)`.
        let parts: Vec<Component> = arguments
            .split(|c: char| c == ',' || c == '/' || c.is_whitespace())
            .filter(|part| !part.is_empty())
            .map(Component::parse)
            .collect::<Option<_>>()?;
        let (&[first, second, third], rest) = parts.split_first_chunk::<3>()?;
        let alpha = match rest {
            [] => 1.0,
            [alpha] => alpha.alpha(),
            _ => return None,
        };

        match function {
            "rgb" | "rgba" => {
                // `rgb(1.0, 0.5, 0)` is written in 0 to 1, as shaders and
                // design tools do; `rgb(1, 0, 0)` is still 0 to 255.
                let unit = if Component::fractions(&[first, second, third]) {
                    255.0
                } else {
                    1.0
                };
                let channel = |c: Component| match c.unit {
                    Unit::Percent => byte(c.value / 100.0 * 255.0),
                    _ => byte(c.value * unit),
                };
                Some(Self {
                    red: channel(first),
                    green: channel(second),
                    blue: channel(third),
                    alpha,
                })
            }
            "hsl" | "hsla" => {
                // Saturation and lightness as `0.5` rather than `50%`.
                let unit = if Component::fractions(&[second, third]) {
                    1.0
                } else {
                    100.0
                };
                let share = |c: Component| match c.unit {
                    Unit::Percent => c.value / 100.0,
                    _ => c.value / unit,
                };
                Some(from_hsl(
                    first.degrees(),
                    share(second),
                    share(third),
                    alpha,
                ))
            }
            "oklch" => {
                // Lightness is 0 to 1, or 0 to 100 when written above 1.
                let lightness = match first.unit {
                    Unit::Percent => first.value / 100.0,
                    _ if first.value > 1.0 => first.value / 100.0,
                    _ => first.value,
                };
                Some(from_oklch(
                    lightness.clamp(0.0, 1.0),
                    // 100% chroma is defined as 0.4.
                    second.scaled(0.4).max(0.0),
                    third.degrees(),
                    alpha,
                ))
            }
            _ => None,
        }
    }

    /// `#rrggbb`, with the alpha appended unless opaque.
    pub fn to_hex(self) -> String {
        let Self {
            red, green, blue, ..
        } = self;
        let mut hex = format!("#{red:02x}{green:02x}{blue:02x}");
        if self.alpha < 1.0 {
            hex.push_str(&format!("{:02x}", byte(self.alpha * 255.0)));
        }
        hex
    }

    pub fn to_rgb(self) -> String {
        let Self {
            red, green, blue, ..
        } = self;
        match self.alpha_suffix() {
            Some(alpha) => format!("rgb({red} {green} {blue} / {alpha})"),
            None => format!("rgb({red}, {green}, {blue})"),
        }
    }

    pub fn to_hsl(self) -> String {
        let [red, green, blue] = [self.red, self.green, self.blue].map(|c| f32::from(c) / 255.0);
        let max = red.max(green).max(blue);
        let min = red.min(green).min(blue);
        let lightness = (max + min) / 2.0;
        let delta = max - min;
        let (hue, saturation) = if delta == 0.0 {
            (0.0, 0.0)
        } else {
            let saturation = delta / (1.0 - (2.0 * lightness - 1.0).abs());
            let sector = if max == red {
                ((green - blue) / delta).rem_euclid(6.0)
            } else if max == green {
                (blue - red) / delta + 2.0
            } else {
                (red - green) / delta + 4.0
            };
            (sector * 60.0, saturation)
        };
        let (hue, saturation, lightness) = (
            hue.round() % 360.0,
            (saturation * 100.0).round(),
            (lightness * 100.0).round(),
        );
        match self.alpha_suffix() {
            Some(alpha) => format!("hsl({hue} {saturation}% {lightness}% / {alpha})"),
            None => format!("hsl({hue}, {saturation}%, {lightness}%)"),
        }
    }

    pub fn to_oklch(self) -> String {
        let [red, green, blue] = [self.red, self.green, self.blue].map(linear);
        let l = (0.412_221_46 * red + 0.536_332_55 * green + 0.051_445_995 * blue).cbrt();
        let m = (0.211_903_5 * red + 0.680_699_5 * green + 0.107_396_96 * blue).cbrt();
        let s = (0.088_302_46 * red + 0.281_718_85 * green + 0.629_978_7 * blue).cbrt();
        let lightness = 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s;
        let a = 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s;
        let b = 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s;
        let chroma = a.hypot(b);
        // The hue of a gray is meaningless noise.
        let hue = if chroma < 0.000_5 {
            0.0
        } else {
            b.atan2(a).to_degrees().rem_euclid(360.0)
        };
        let alpha = self
            .alpha_suffix()
            .map(|alpha| format!(" / {alpha}"))
            .unwrap_or_default();
        format!("oklch({lightness:.3} {chroma:.3} {hue:.1}{alpha})")
    }

    fn alpha_suffix(self) -> Option<String> {
        (self.alpha < 1.0).then(|| format!("{}%", (self.alpha * 100.0).round()))
    }

    /// The WCAG contrast ratio between this color, ignoring alpha, and gray of
    /// the given linear luminance.
    fn contrast_with(self, luminance: f32) -> f32 {
        let own =
            0.2126 * linear(self.red) + 0.7152 * linear(self.green) + 0.0722 * linear(self.blue);
        let (light, dark) = (own.max(luminance), own.min(luminance));
        (light + 0.05) / (dark + 0.05)
    }

    pub fn contrast_with_white(self) -> f32 {
        self.contrast_with(1.0)
    }

    pub fn contrast_with_black(self) -> f32 {
        self.contrast_with(0.0)
    }

    /// `0xRRGGBBAA`.
    pub fn to_rgba_u32(self) -> u32 {
        u32::from_be_bytes([self.red, self.green, self.blue, byte(self.alpha * 255.0)])
    }
}

/// One argument of a color function.
#[derive(Clone, Copy)]
struct Component {
    value: f32,
    unit: Unit,
    /// Written with a decimal point, like `0.5` or `1.0`.
    fractional: bool,
}

#[derive(Clone, Copy)]
enum Unit {
    None,
    Percent,
    Degrees,
    Turns,
    Radians,
}

impl Component {
    fn parse(text: &str) -> Option<Self> {
        if text == "none" {
            return Some(Self {
                value: 0.0,
                unit: Unit::None,
                fractional: false,
            });
        }
        let (number, unit) = [
            ("%", Unit::Percent),
            ("deg", Unit::Degrees),
            ("turn", Unit::Turns),
            ("rad", Unit::Radians),
        ]
        .into_iter()
        .find_map(|(suffix, unit)| Some((text.strip_suffix(suffix)?, unit)))
        .unwrap_or((text, Unit::None));
        let value: f32 = number.parse().ok()?;
        let fractional = number.contains('.');
        value.is_finite().then_some(Self {
            value,
            unit,
            fractional,
        })
    }

    /// Whether unitless components are fractions of 1 rather than of 255 or
    /// 100: all of them at most 1, and at least one written with a decimal
    /// point.
    fn fractions(components: &[Self]) -> bool {
        let unitless: Vec<_> = components
            .iter()
            .filter(|c| matches!(c.unit, Unit::None))
            .collect();
        unitless.iter().all(|c| c.value <= 1.0) && unitless.iter().any(|c| c.fractional)
    }

    /// An alpha: `50%`, `0.5`, or a byte like `128` as in `rgba(255 0 0 128)`.
    fn alpha(self) -> f32 {
        let alpha = match self.unit {
            Unit::Percent => self.value / 100.0,
            _ if self.value > 1.0 && !self.fractional => self.value / 255.0,
            _ => self.value,
        };
        alpha.clamp(0.0, 1.0)
    }

    /// The value, with percentages taken of `whole`.
    fn scaled(self, whole: f32) -> f32 {
        match self.unit {
            Unit::Percent => self.value / 100.0 * whole,
            _ => self.value,
        }
    }

    fn degrees(self) -> f32 {
        let degrees = match self.unit {
            Unit::Turns => self.value * 360.0,
            Unit::Radians => self.value * 180.0 / PI,
            _ => self.value,
        };
        degrees.rem_euclid(360.0)
    }
}

fn byte(value: f32) -> u8 {
    value.round().clamp(0.0, 255.0) as u8
}

fn parse_hex(digits: &str) -> Option<Color> {
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = |range: std::ops::Range<usize>| u8::from_str_radix(&digits[range], 16).ok();
    let [red, green, blue, alpha] = match digits.len() {
        3 | 4 => {
            let short = |i: usize| value(i..i + 1).map(|v| v * 17);
            let alpha = if digits.len() == 4 { short(3)? } else { 255 };
            [short(0)?, short(1)?, short(2)?, alpha]
        }
        6 | 8 => {
            let alpha = if digits.len() == 8 { value(6..8)? } else { 255 };
            [value(0..2)?, value(2..4)?, value(4..6)?, alpha]
        }
        _ => return None,
    };
    Some(Color {
        red,
        green,
        blue,
        alpha: f32::from(alpha) / 255.0,
    })
}

fn from_hsl(hue: f32, saturation: f32, lightness: f32, alpha: f32) -> Color {
    let saturation = saturation.clamp(0.0, 1.0);
    let lightness = lightness.clamp(0.0, 1.0);
    let amplitude = saturation * lightness.min(1.0 - lightness);
    let channel = |offset: f32| {
        let k = (offset + hue / 30.0) % 12.0;
        byte(255.0 * (lightness - amplitude * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0)))
    };
    Color {
        red: channel(0.0),
        green: channel(8.0),
        blue: channel(4.0),
        alpha,
    }
}

fn from_oklch(lightness: f32, chroma: f32, hue: f32, alpha: f32) -> Color {
    let (sin, cos) = hue.to_radians().sin_cos();
    let (a, b) = (chroma * cos, chroma * sin);
    let l = (lightness + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m = (lightness - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s = (lightness - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    Color {
        red: encoded(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
        green: encoded(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
        blue: encoded(-0.004_196_086 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        alpha,
    }
}

/// sRGB byte to linear light.
fn linear(channel: u8) -> f32 {
    let value = f32::from(channel) / 255.0;
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light to an sRGB byte, clipping what the gamut cannot show.
fn encoded(value: f32) -> u8 {
    let value = value.clamp(0.0, 1.0);
    let encoded = if value <= 0.003_130_8 {
        12.92 * value
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    };
    byte(255.0 * encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(text: &str) -> Option<String> {
        Color::parse(text).map(Color::to_hex)
    }

    #[test]
    fn accepts_css_colors() {
        for (text, expected) in [
            ("#fff", "#ffffff"),
            ("#FF7A1A", "#ff7a1a"),
            ("#ff7a1a80", "#ff7a1a80"),
            ("#f008", "#ff000088"),
            ("rgb(255, 122, 26)", "#ff7a1a"),
            ("rgba(255,122,26,0.5)", "#ff7a1a80"),
            ("rgb(100% 0% 0% / 50%)", "#ff000080"),
            ("hsl(0deg 100% 50%)", "#ff0000"),
            ("hsl(120, 100%, 25%)", "#008000"),
            ("hsl(0.5turn 100% 50%)", "#00ffff"),
            ("oklch(1 0 0)", "#ffffff"),
            ("oklch(0.62796 0.25768 29.234)", "#ff0000"),
            ("oklch(62.796% 0.25768 29.234)", "#ff0000"),
            ("oklch(62.796 0.25768 29.234)", "#ff0000"),
            ("oklch(0.62796 0.25768 29.234 / 0.5)", "#ff000080"),
            // 0 to 1 floats.
            ("rgb(1.0, 0.478, 0.102)", "#ff7a1a"),
            ("rgba(1.0, 0, 0, 0.5)", "#ff000080"),
            ("rgb(0.5 0.5 0.5)", "#808080"),
            // Integers stay 0 to 255, even when small.
            ("rgb(1, 0, 0)", "#010000"),
            ("rgb(1, 1, 1)", "#010101"),
            // Percentages, also mixed with numbers.
            ("rgb(100%, 50%, 0%)", "#ff8000"),
            ("rgb(100% 128 0)", "#ff8000"),
            // Alpha as a fraction, a percentage or a byte.
            ("rgba(255, 0, 0, .5)", "#ff000080"),
            ("rgba(255, 0, 0, 50%)", "#ff000080"),
            ("rgba(255, 0, 0, 128)", "#ff000080"),
            ("rgba(255, 0, 0, 1)", "#ff0000"),
            ("rgba(255, 0, 0, 0)", "#ff000000"),
            ("rgb(255 0 0 / 0.25)", "#ff000040"),
            // HSL saturation and lightness as numbers or fractions.
            ("hsl(120 100 25)", "#008000"),
            ("hsl(120, 1.0, 0.25)", "#008000"),
            ("hsla(0, 100%, 50%, 0.5)", "#ff000080"),
            ("hsl(0 100% 50% / 128)", "#ff000080"),
        ] {
            assert_eq!(hex(text).as_deref(), Some(expected), "{text}");
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
            "#ééé",
            "ff7a1a",
            "red",
            "rgb(1, 2)",
            "rgb(a, b, c)",
            "rgb(1, 2, 3, 4, 5)",
            "rgb(1, 2, 3) and more",
            "print(1, 2, 3)",
            "# heading",
        ] {
            assert_eq!(Color::parse(text), None, "{text:?} was accepted");
        }
    }

    #[test]
    fn converts_between_notations() {
        let orange = Color::parse("#ff7a1a").unwrap();
        assert_eq!(orange.to_rgb(), "rgb(255, 122, 26)");
        assert_eq!(orange.to_hsl(), "hsl(25, 100%, 55%)");
        assert_eq!(hex(&orange.to_oklch()).as_deref(), Some("#ff7a1a"));
        let faded = Color::parse("#00000080").unwrap();
        assert_eq!(faded.to_rgb(), "rgb(0 0 0 / 50%)");
        assert_eq!(faded.to_hsl(), "hsl(0 0% 0% / 50%)");
        assert_eq!(faded.to_oklch(), "oklch(0.000 0.000 0.0 / 50%)");
        assert_eq!(faded.to_rgba_u32(), 0x0000_0080);
    }

    #[test]
    fn measures_contrast() {
        let white = Color::parse("#fff").unwrap();
        assert!((white.contrast_with_black() - 21.0).abs() < 0.01);
        assert!((white.contrast_with_white() - 1.0).abs() < 0.01);
    }
}

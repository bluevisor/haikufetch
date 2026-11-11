//! Terminal colors: a tiny ANSI writer, color parsing and the palette haikufetch draws with.
//!
//! Colors are stored as what the user asked for (an ANSI slot or an RGB triple) and only turned
//! into escape codes at the end, by a `Painter` that knows whether this run uses color at all and
//! whether the terminal shows 24-bit color. Without 24-bit color, RGB becomes the nearest entry of
//! the 256-color cube, which every terminal of the last 20 years understands.

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    /// One of the 16 terminal colors (0-7 normal, 8-15 bright), drawn in the user's theme
    Ansi(u8),
    Rgb(u8, u8, u8),
}

/// The accent colors haikufetch uses for usage figures, the rainbow logo and the Apple key color
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Accent {
    Green,
    Yellow,
    Orange,
    Red,
    Magenta,
    Blue,
}

impl Accent {
    /// Soft 24-bit tones for the usage percentages and bars, easier on the eye than the saturated
    /// ANSI colors many themes ship
    pub fn muted(self) -> Color {
        match self {
            Accent::Green => Color::Rgb(0x8f, 0xbf, 0x7f),
            Accent::Yellow => Color::Rgb(0xe3, 0xc4, 0x7a),
            Accent::Orange => Color::Rgb(0xe0, 0xa0, 0x6f),
            Accent::Red => Color::Rgb(0xd4, 0x84, 0x7f),
            Accent::Magenta => Color::Rgb(0xae, 0x8a, 0xb8),
            Accent::Blue => Color::Rgb(0x77, 0xaf, 0xd4),
        }
    }

    /// The stripes of Apple's 1977 six-color logo, for the rainbow logo where muted tones look
    /// washed out
    pub fn vivid(self) -> Color {
        match self {
            Accent::Green => Color::Rgb(0x61, 0xbb, 0x46),
            Accent::Yellow => Color::Rgb(0xfd, 0xb8, 0x27),
            Accent::Orange => Color::Rgb(0xf5, 0x82, 0x1f),
            Accent::Red => Color::Rgb(0xe0, 0x3a, 0x3e),
            Accent::Magenta => Color::Rgb(0x96, 0x3d, 0x97),
            Accent::Blue => Color::Rgb(0x00, 0x9d, 0xdc),
        }
    }

    /// The theme's own color, used when the terminal can't show 24-bit color. ANSI has no orange,
    /// bright red stands in
    pub fn ansi(self) -> Color {
        match self {
            Accent::Green => Color::Ansi(2),
            Accent::Yellow => Color::Ansi(3),
            Accent::Orange => Color::Ansi(9),
            Accent::Red => Color::Ansi(1),
            Accent::Magenta => Color::Ansi(5),
            Accent::Blue => Color::Ansi(4),
        }
    }
}

/// The dim track behind a usage bar's filled part
pub const TRACK: Color = Color::Rgb(0x4a, 0x4a, 0x4a);
pub const TRACK_ANSI: Color = Color::Ansi(8);

const NAMES: [&str; 8] = [
    "black", "red", "green", "yellow", "blue", "magenta", "cyan", "white",
];

/// Parses a color name ("red", "bright blue", "bright-blue", "gray") or a hex code ("#ff8800",
/// "#f80"). None for anything else, so callers can decide what an unknown value means
pub fn parse_color(value: &str) -> Option<Color> {
    let value = value.trim().to_ascii_lowercase();
    if let Some(hex) = value.strip_prefix('#') {
        return parse_hex(hex);
    }
    let words: String = value
        .split(|c: char| c.is_whitespace() || c == '-' || c == '_')
        .collect::<Vec<_>>()
        .join(" ");
    let (bright, base) = match words.strip_prefix("bright ") {
        Some(base) => (true, base),
        None => (false, words.as_str()),
    };
    let index = match base {
        "gray" | "grey" if !bright => return Some(Color::Ansi(8)),
        "purple" => 5,
        name => NAMES.iter().position(|n| *n == name)?,
    };
    Some(Color::Ansi(index as u8 + if bright { 8 } else { 0 }))
}

fn parse_hex(hex: &str) -> Option<Color> {
    if !hex.is_ascii() {
        return None;
    }
    let digits: Vec<u8> = match hex.len() {
        3 => hex
            .chars()
            .map(|c| c.to_digit(16).map(|d| d as u8 * 17))
            .collect::<Option<_>>()?,
        6 => (0..3)
            .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
            .collect::<Option<_>>()?,
        _ => return None,
    };
    Some(Color::Rgb(digits[0], digits[1], digits[2]))
}

/// Maps an RGB color to the nearest entry of xterm's 6x6x6 color cube or its gray ramp
pub fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    const STEPS: [i32; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (STEPS[i] - v as i32).abs())
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (STEPS[ri], STEPS[gi], STEPS[bi]);
    let distance = |c: (i32, i32, i32)| {
        (c.0 - r as i32).pow(2) + (c.1 - g as i32).pow(2) + (c.2 - b as i32).pow(2)
    };

    let average = (r as i32 + g as i32 + b as i32) / 3;
    let gray_index = ((average - 8).max(0) / 10).min(23);
    let gray = 8 + gray_index * 10;

    if distance((gray, gray, gray)) < distance(cube) {
        232 + gray_index as u8
    } else {
        16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8
    }
}

/// Checks the environment for 24-bit color support. COLORTERM is the standard signal, a few
/// terminals that support it without saying so are recognized by name
pub fn detect_truecolor(colorterm: Option<&str>, term_program: Option<&str>) -> bool {
    matches!(colorterm, Some("truecolor" | "24bit"))
        || matches!(
            term_program,
            Some("iTerm.app" | "WezTerm" | "ghostty" | "vscode" | "Hyper" | "Tabby" | "rio")
        )
}

/// Turns colors into escape codes for this run
#[derive(Debug, Clone, Copy)]
pub struct Painter {
    pub enabled: bool,
    pub truecolor: bool,
}

impl Painter {
    pub const PLAIN: Painter = Painter {
        enabled: false,
        truecolor: false,
    };

    /// Picks the muted 24-bit tone of an accent, or the theme's ANSI color without 24-bit color
    pub fn accent(&self, accent: Accent) -> Color {
        if self.truecolor {
            accent.muted()
        } else {
            accent.ansi()
        }
    }

    /// Picks the saturated tone of an accent, or the theme's ANSI color without 24-bit color
    pub fn vivid(&self, accent: Accent) -> Color {
        if self.truecolor {
            accent.vivid()
        } else {
            accent.ansi()
        }
    }

    pub fn track(&self) -> Color {
        if self.truecolor { TRACK } else { TRACK_ANSI }
    }

    /// Appends the escape code that switches to a foreground color
    pub fn push_fg(&self, out: &mut String, color: Color) {
        if !self.enabled {
            return;
        }
        let _ = match color {
            Color::Ansi(n) if n < 8 => write!(out, "\x1b[{}m", 30 + n),
            Color::Ansi(n) => write!(out, "\x1b[{}m", 90 + (n & 7)),
            Color::Rgb(r, g, b) if self.truecolor => write!(out, "\x1b[38;2;{r};{g};{b}m"),
            Color::Rgb(r, g, b) => write!(out, "\x1b[38;5;{}m", rgb_to_256(r, g, b)),
        };
    }

    pub fn push_bold(&self, out: &mut String) {
        if self.enabled {
            out.push_str("\x1b[1m");
        }
    }

    pub fn push_reset(&self, out: &mut String) {
        if self.enabled {
            out.push_str("\x1b[0m");
        }
    }

    /// Returns text in a color, followed by a reset
    pub fn paint(&self, text: &str, color: Color) -> String {
        let mut out = String::with_capacity(text.len() + 24);
        self.push_fg(&mut out, color);
        out.push_str(text);
        self.push_reset(&mut out);
        out
    }

    pub fn bold(&self, text: &str) -> String {
        let mut out = String::with_capacity(text.len() + 8);
        self.push_bold(&mut out);
        out.push_str(text);
        self.push_reset(&mut out);
        out
    }
}

/// Counts the columns a string takes on screen, skipping escape codes. Everything haikufetch prints
/// is single-width (ASCII, block elements, box drawing), so a char is a column
pub fn visible_width(text: &str) -> usize {
    let mut width = 0;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // CSI sequences end at their first letter
            for c in chars.by_ref() {
                if c.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            width += 1;
        }
    }
    width
}

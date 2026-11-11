//! Logos: the built-in art, which one to draw, and how to color it.
//!
//! Art uses fastfetch's markers: `$1` .. `$9` switch to the logo's 1st .. 9th color, and text
//! before the first marker takes color 1. A custom logo file can use the same markers, or carry
//! its own escape codes.

use std::path::Path;

use crate::style::{Accent, Color, Painter};

pub struct Logo {
    pub id: &'static str,
    /// Other ids (os-release ID or ID_LIKE values) that pick this logo
    pub aliases: &'static [&'static str],
    pub art: &'static str,
    /// $1, $2, ... in 24-bit and their ANSI stand-ins
    pub colors: &'static [(Color, Color)],
}

const fn rgb(r: u8, g: u8, b: u8, ansi: u8) -> (Color, Color) {
    (Color::Rgb(r, g, b), Color::Ansi(ansi))
}
const fn ansi(n: u8) -> (Color, Color) {
    (Color::Ansi(n), Color::Ansi(n))
}

pub const LOGOS: &[Logo] = &[
    Logo {
        id: "macos",
        aliases: &["darwin", "mac"],
        art: include_str!("../ascii/macos.txt"),
        colors: &[ansi(7)],
    },
    Logo {
        id: "haiku",
        aliases: &[],
        art: include_str!("../ascii/haiku.txt"),
        colors: &[
            rgb(236, 236, 236, 15), // the pillars
            rgb(150, 150, 150, 7),  // their shaded edge
            rgb(98, 189, 36, 10),   // the leaf
            rgb(44, 120, 22, 2),    // its dim edge
        ],
    },
    Logo {
        id: "arch",
        aliases: &["archarm", "artix"],
        art: include_str!("../ascii/arch.txt"),
        colors: &[rgb(23, 147, 209, 6)],
    },
    Logo {
        id: "ubuntu",
        aliases: &["kubuntu", "xubuntu", "lubuntu", "neon"],
        art: include_str!("../ascii/ubuntu.txt"),
        colors: &[rgb(233, 84, 32, 1), ansi(15)],
    },
    Logo {
        id: "debian",
        aliases: &["raspbian"],
        art: include_str!("../ascii/debian.txt"),
        colors: &[rgb(215, 10, 83, 1)],
    },
    Logo {
        id: "fedora",
        aliases: &["rhel", "centos", "rocky", "almalinux", "nobara"],
        art: include_str!("../ascii/fedora.txt"),
        colors: &[rgb(81, 162, 218, 4)],
    },
    Logo {
        id: "manjaro",
        aliases: &[],
        art: include_str!("../ascii/manjaro.txt"),
        colors: &[rgb(53, 191, 92, 2)],
    },
    Logo {
        id: "linuxmint",
        aliases: &["mint"],
        art: include_str!("../ascii/linuxmint.txt"),
        colors: &[rgb(135, 207, 62, 2), ansi(15)],
    },
    Logo {
        id: "opensuse",
        aliases: &["opensuse-tumbleweed", "opensuse-leap", "suse", "sles"],
        art: include_str!("../ascii/opensuse.txt"),
        colors: &[rgb(115, 186, 37, 2)],
    },
    Logo {
        id: "alpine",
        aliases: &[],
        art: include_str!("../ascii/alpine.txt"),
        colors: &[rgb(13, 89, 127, 4)],
    },
    Logo {
        id: "gentoo",
        aliases: &["funtoo"],
        art: include_str!("../ascii/gentoo.txt"),
        colors: &[rgb(84, 73, 149, 5)],
    },
    Logo {
        id: "endeavouros",
        aliases: &[],
        art: include_str!("../ascii/endeavouros.txt"),
        colors: &[
            rgb(255, 69, 0, 1),
            rgb(127, 63, 191, 5),
            rgb(23, 147, 209, 4),
        ],
    },
    Logo {
        id: "popos",
        aliases: &["pop"],
        art: include_str!("../ascii/popos.txt"),
        colors: &[rgb(72, 185, 199, 6)],
    },
    Logo {
        id: "cachyos",
        aliases: &[],
        art: include_str!("../ascii/cachyos.txt"),
        colors: &[rgb(0, 184, 148, 6)],
    },
    Logo {
        id: "garuda",
        aliases: &[],
        art: include_str!("../ascii/garuda.txt"),
        colors: &[rgb(138, 43, 226, 5)],
    },
    Logo {
        id: "kali",
        aliases: &[],
        art: include_str!("../ascii/kali.txt"),
        colors: &[rgb(38, 139, 210, 4)],
    },
    Logo {
        id: "zorin",
        aliases: &[],
        art: include_str!("../ascii/zorin.txt"),
        colors: &[rgb(17, 162, 236, 4)],
    },
    Logo {
        id: "elementary",
        aliases: &[],
        art: include_str!("../ascii/elementary.txt"),
        colors: &[ansi(15)],
    },
    Logo {
        id: "nixos",
        aliases: &["nix"],
        art: include_str!("../ascii/nixos.txt"),
        colors: &[rgb(125, 176, 221, 4)],
    },
    Logo {
        id: "linux",
        aliases: &[],
        art: include_str!("../ascii/linux.txt"),
        colors: &[ansi(15), ansi(3)],
    },
];

pub fn find(id: &str) -> Option<&'static Logo> {
    let id = id.trim().to_ascii_lowercase();
    LOGOS
        .iter()
        .find(|l| l.id == id || l.aliases.contains(&id.as_str()))
}

/// Picks the system's logo from its id and the ids it is like (os-release ID_LIKE), falling back
/// to Tux on an unknown Linux
pub fn for_system(id: &str, like: &[String]) -> Option<&'static Logo> {
    find(id)
        .or_else(|| like.iter().find_map(|l| find(l)))
        .or_else(|| cfg!(target_os = "linux").then(|| find("linux")).flatten())
}

/// How the [logo] color setting asks for the logo to be drawn
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    Default,
    Rainbow,
    Solid(Color),
}

pub fn parse_tint(value: &str) -> Tint {
    match value.trim().to_ascii_lowercase().as_str() {
        "rainbow" => Tint::Rainbow,
        other => crate::style::parse_color(other)
            .map(Tint::Solid)
            .unwrap_or(Tint::Default),
    }
}

/// What's drawn: colored lines and the width of the widest one
pub struct Rendered {
    pub lines: Vec<String>,
    pub width: usize,
}

/// Colors a logo's art. Rainbow only means something for the Apple logo, others keep their colors
pub fn render(
    art: &str,
    colors: &[(Color, Color)],
    is_apple: bool,
    tint: Tint,
    painter: &Painter,
) -> Rendered {
    let pick = |index: usize| -> Option<Color> {
        if let Tint::Solid(color) = tint {
            return Some(color);
        }
        let (true_color, ansi) = colors.get(index).or(colors.first()).copied()?;
        Some(if painter.truecolor { true_color } else { ansi })
    };

    let mut lines = Vec::new();
    let mut width = 0;
    for (row, line) in art.lines().enumerate() {
        let stripe = (is_apple && tint == Tint::Rainbow).then(|| painter.vivid(apple_stripe(row)));
        let mut out = String::new();
        let mut visible = String::new();
        let mut chars = line.chars().peekable();
        if let Some(color) = stripe.or_else(|| pick(0)) {
            painter.push_fg(&mut out, color);
        }
        while let Some(c) = chars.next() {
            if c == '$'
                && let Some(digit) = chars.peek().and_then(|d| d.to_digit(10))
                && digit > 0
            {
                chars.next();
                if stripe.is_none()
                    && let Some(color) = pick(digit as usize - 1)
                {
                    painter.push_fg(&mut out, color);
                }
                continue;
            }
            out.push(c);
            visible.push(c);
        }
        painter.push_reset(&mut out);
        width = width.max(crate::style::visible_width(&visible));
        lines.push(out);
    }
    Rendered { lines, width }
}

/// The six stripes of the classic Apple logo over its 19 rows: green covers the leaf and the top,
/// every other stripe gets two rows
fn apple_stripe(row: usize) -> Accent {
    match row {
        0..=7 => Accent::Green,
        8 | 9 => Accent::Yellow,
        10 | 11 => Accent::Orange,
        12 | 13 => Accent::Red,
        14 | 15 => Accent::Magenta,
        _ => Accent::Blue,
    }
}

/// Loads a logo from a text file, for `logo = "~/art.txt"`
pub fn load_file(path: &str) -> Option<String> {
    let expanded = match path.strip_prefix("~/") {
        Some(rest) => crate::paths::home()?.join(rest),
        None => Path::new(path).to_path_buf(),
    };
    std::fs::read_to_string(expanded).ok()
}

/// Where custom logos picked by name live: ~/.config/haikufetch/logos/<name>.txt
pub fn user_logo_dir() -> Option<std::path::PathBuf> {
    crate::paths::config_dir().map(|dir| dir.join("haikufetch/logos"))
}

/// A custom logo by name from the logos folder
pub fn user_logo(name: &str) -> Option<String> {
    if name.contains('/') || name.contains('.') {
        return None;
    }
    std::fs::read_to_string(user_logo_dir()?.join(format!("{name}.txt"))).ok()
}

/// The names of the custom logos in the logos folder
pub fn user_logos() -> Vec<String> {
    let mut names: Vec<String> = user_logo_dir()
        .and_then(|dir| std::fs::read_dir(dir).ok())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_string_lossy()
                .strip_suffix(".txt")
                .map(str::to_string)
        })
        .collect();
    names.sort();
    names
}

/// Checks if a logo setting names a file rather than a built-in logo
pub fn is_path(value: &str) -> bool {
    value.contains('/') || value.ends_with(".txt")
}

/// The color the labels take with key_color = "auto": a solid logo color when one is set, the
/// Apple logo's yellow stripe next to the Apple logo (fastfetch does the same), and the logo's
/// main color everywhere else
pub fn auto_key_color(logo: Option<&Logo>, tint: Tint, painter: &Painter) -> Option<Color> {
    if let Tint::Solid(color) = tint {
        return Some(color);
    }
    let logo = logo?;
    if logo.id == "macos" {
        return Some(painter.vivid(Accent::Yellow));
    }
    // Haiku's first color is the white pillars; its leaf green reads better as a label color
    let index = if logo.id == "haiku" { 2 } else { 0 };
    let (true_color, ansi) = logo.colors.get(index).copied()?;
    Some(if painter.truecolor { true_color } else { ansi })
}

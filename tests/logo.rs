#![allow(clippy::unwrap_used, clippy::expect_used)]

use haikufetch::{
    logo::{self, Tint},
    style::{Color, Painter, visible_width},
};

const COLOR: Painter = Painter {
    enabled: true,
    truecolor: true,
};

#[test]
fn every_logo_renders_without_leftover_markers() {
    for l in logo::LOGOS {
        let plain = logo::render(
            l.art,
            l.colors,
            l.id == "macos",
            Tint::Default,
            &Painter::PLAIN,
        );
        assert!(!plain.lines.is_empty(), "{}", l.id);
        for line in &plain.lines {
            assert!(
                !line.contains("$1") && !line.contains("$2"),
                "{}: {line}",
                l.id
            );
            assert!(visible_width(line) <= plain.width);
        }
        // the colored version has the same shape
        let colored = logo::render(l.art, l.colors, l.id == "macos", Tint::Default, &COLOR);
        assert_eq!(colored.width, plain.width, "{}", l.id);
        assert_eq!(colored.lines.len(), plain.lines.len());
    }
}

#[test]
fn every_marker_has_a_color() {
    for l in logo::LOGOS {
        for (i, _) in l.art.match_indices('$') {
            let digit = l.art[i + 1..]
                .chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .unwrap_or(0);
            assert!(
                digit >= 1 && digit as usize <= l.colors.len(),
                "{} uses ${digit}",
                l.id
            );
        }
    }
}

#[test]
fn the_apple_logo_keeps_its_shape() {
    let mac = logo::find("macos").unwrap();
    assert_eq!(mac.art.lines().count(), 19);
    assert!(mac.art.lines().all(|l| !l.ends_with(' ')));
    let rendered = logo::render(mac.art, mac.colors, true, Tint::Default, &Painter::PLAIN);
    assert!(rendered.width <= 30);
}

#[test]
fn rainbow_paints_the_apple_stripes() {
    let mac = logo::find("macos").unwrap();
    let rainbow = logo::render(mac.art, mac.colors, true, Tint::Rainbow, &COLOR);
    assert!(rainbow.lines[0].contains("38;2;97;187;70"), "green leaf");
    assert!(rainbow.lines[18].contains("38;2;0;157;220"), "blue bottom");
}

#[test]
fn a_solid_tint_paints_everything() {
    let arch = logo::find("arch").unwrap();
    let red = logo::render(
        arch.art,
        arch.colors,
        false,
        Tint::Solid(Color::Ansi(1)),
        &COLOR,
    );
    assert!(red.lines.iter().all(|l| l.starts_with("\x1b[31m")));
}

#[test]
fn ids_and_aliases_pick_logos() {
    assert_eq!(logo::find("pop").map(|l| l.id), Some("popos"));
    assert_eq!(
        logo::find("opensuse-tumbleweed").map(|l| l.id),
        Some("opensuse")
    );
    assert_eq!(logo::find("ARCH").map(|l| l.id), Some("arch"));
    assert_eq!(logo::for_system("neon", &[]).map(|l| l.id), Some("ubuntu"));
    // an unknown derivative falls back to what it is like
    assert_eq!(
        logo::for_system("someos", &[String::from("debian")]).map(|l| l.id),
        Some("debian")
    );
}

#[test]
fn tints_parse() {
    assert_eq!(logo::parse_tint("rainbow"), Tint::Rainbow);
    assert_eq!(logo::parse_tint("default"), Tint::Default);
    assert_eq!(logo::parse_tint("#1é234"), Tint::Default);
    assert_eq!(logo::parse_tint("bright red"), Tint::Solid(Color::Ansi(9)));
}

#[test]
fn auto_key_colors_follow_the_logo() {
    let mac = logo::find("macos");
    assert_eq!(
        logo::auto_key_color(mac, Tint::Default, &COLOR),
        Some(Color::Rgb(0xfd, 0xb8, 0x27))
    );
    assert_eq!(
        logo::auto_key_color(
            mac,
            Tint::Default,
            &Painter {
                enabled: true,
                truecolor: false
            }
        ),
        Some(Color::Ansi(3))
    );
    assert_eq!(
        logo::auto_key_color(logo::find("haiku"), Tint::Default, &COLOR),
        Some(Color::Rgb(98, 189, 36))
    );
    assert_eq!(
        logo::auto_key_color(None, Tint::Solid(Color::Ansi(4)), &COLOR),
        Some(Color::Ansi(4))
    );
    assert_eq!(logo::auto_key_color(None, Tint::Default, &COLOR), None);
}

#[test]
fn logo_paths_are_told_from_names() {
    assert!(logo::is_path("~/art.txt"));
    assert!(logo::is_path("/etc/logo"));
    assert!(logo::is_path("art.txt"));
    assert!(!logo::is_path("arch"));
}

#[test]
fn custom_logo_names_cant_escape_the_logos_folder() {
    assert!(logo::user_logo("../config").is_none());
    assert!(logo::user_logo("x.txt").is_none());
}

#[test]
fn the_seen_example_uses_three_leaves_and_their_edge_shades() {
    let art = include_str!("../examples/logos/seen.txt");
    let colors = [(Color::Ansi(10), Color::Ansi(10)); 6];
    let rendered = logo::render(art, &colors, false, Tint::Default, &COLOR);
    assert_eq!(rendered.lines.len(), 14);
    assert_eq!(rendered.width, 32);
    for marker in ["$1", "$2", "$3", "$4", "$5", "$6"] {
        assert!(art.contains(marker));
    }
    assert!(!art.contains("$7"));
}

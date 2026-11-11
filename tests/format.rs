#![allow(clippy::unwrap_used, clippy::expect_used)]

use haikufetch::{
    cli::{self, ColorWhen},
    format::*,
    style::{self, Color, Painter},
};

#[test]
fn sizes_use_binary_units_and_drop_trailing_zeros() {
    assert_eq!(size(0), "0 B");
    assert_eq!(size(1023), "1023 B");
    assert_eq!(size(1024), "1 KiB");
    assert_eq!(size(64 * 1024 * 1024 * 1024), "64 GiB");
    assert_eq!(size(3_785_486_336), "3.52 GiB");
    assert_eq!(size(55_803_232_256), "51.9 GiB");
    assert_eq!(size(862_930_632_704), "803 GiB");
    assert_eq!(size(2 * 1024u64.pow(4)), "2 TiB");
}

#[test]
fn sizes_round_down_so_nearly_full_never_reads_as_full() {
    // 63.99 GiB
    assert_eq!(size(68_708_000_000), "63.9 GiB");
}

#[test]
fn durations_read_as_days_hours_minutes() {
    assert_eq!(duration(0), "0s");
    assert_eq!(duration(42), "42s");
    assert_eq!(duration(60), "1m");
    assert_eq!(duration(3600 * 5 + 180), "5h 3m");
    assert_eq!(duration(183_582), "2d 2h 59m");
}

#[test]
fn frequencies() {
    assert_eq!(frequency(0), None);
    assert_eq!(frequency(800).as_deref(), Some("800 MHz"));
    assert_eq!(frequency(999).as_deref(), Some("999 MHz"));
    assert_eq!(frequency(4608).as_deref(), Some("4.61 GHz"));
}

#[test]
fn percentages_floor_and_cap() {
    assert_eq!(percent(1, 0), None);
    assert_eq!(percent(999, 1000), Some(99));
    assert_eq!(percent(2000, 1000), Some(100));
    assert_eq!(percent(u64::MAX, u64::MAX), Some(100));
}

#[test]
fn bars_round_to_eighths_and_keep_their_width() {
    assert_eq!(bar(64, 10), (String::from("██████▍"), String::from("░░░")));
    assert_eq!(bar(0, 10), (String::new(), "░".repeat(10)));
    assert_eq!(bar(100, 10), ("█".repeat(10), String::new()));
    for p in 0..=100 {
        let (filled, empty) = bar(p, 10);
        assert_eq!(filled.chars().count() + empty.chars().count(), 10, "{p}%");
    }
}

#[test]
fn core_counts() {
    assert_eq!(cores(&[6, 12]).as_deref(), Some("6+12"));
    assert_eq!(cores(&[8]).as_deref(), Some("8"));
    assert_eq!(cores(&[6, 0]), None);
    assert_eq!(cores(&[]), None);
}

#[test]
fn cpu_names_lose_marketing_noise() {
    assert_eq!(
        clean_cpu_name("AMD Ryzen 5 5600X 6-Core Processor"),
        "AMD Ryzen 5 5600X"
    );
    assert_eq!(
        clean_cpu_name("Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz"),
        "Intel Core i7-8700K"
    );
    assert_eq!(
        clean_cpu_name("AMD Ryzen 9 7950X3D 16-Core Processor"),
        "AMD Ryzen 9 7950X3D"
    );
    assert_eq!(clean_cpu_name("Apple M5 Pro"), "Apple M5 Pro");
    assert_eq!(
        clean_cpu_name("Intel(R) Xeon(R) CPU E5-2680 v4 @ 2.40GHz"),
        "Intel Xeon"
    );
}

#[test]
fn json_strings_escape_everything_dangerous() {
    assert_eq!(json_string("a\"b\\c\n\u{1b}"), "\"a\\\"b\\\\c\\n\\u001b\"");
}

#[test]
fn colors_parse_names_and_hex() {
    assert_eq!(style::parse_color("red"), Some(Color::Ansi(1)));
    assert_eq!(style::parse_color("Bright Blue"), Some(Color::Ansi(12)));
    assert_eq!(style::parse_color("bright-white"), Some(Color::Ansi(15)));
    assert_eq!(style::parse_color("gray"), Some(Color::Ansi(8)));
    assert_eq!(style::parse_color("#ff8800"), Some(Color::Rgb(255, 136, 0)));
    assert_eq!(style::parse_color("#f80"), Some(Color::Rgb(255, 136, 0)));
    assert_eq!(style::parse_color("#1é234"), None);
    assert_eq!(style::parse_color("auto"), None);
    assert_eq!(style::parse_color(""), None);
}

#[test]
fn rgb_falls_back_to_the_256_color_cube() {
    assert_eq!(style::rgb_to_256(0, 0, 0), 16);
    assert_eq!(style::rgb_to_256(255, 255, 255), 231);
    assert_eq!(style::rgb_to_256(255, 0, 0), 196);
    assert!((232..=255).contains(&style::rgb_to_256(128, 128, 128)));
}

#[test]
fn painter_writes_nothing_when_off() {
    let off = Painter::PLAIN;
    assert_eq!(off.paint("x", Color::Ansi(1)), "x");
    let on = Painter {
        enabled: true,
        truecolor: false,
    };
    assert_eq!(on.paint("x", Color::Ansi(1)), "\x1b[31mx\x1b[0m");
    assert_eq!(on.paint("x", Color::Ansi(9)), "\x1b[91mx\x1b[0m");
    assert_eq!(
        on.paint("x", Color::Rgb(255, 0, 0)),
        "\x1b[38;5;196mx\x1b[0m"
    );
    let true_color = Painter {
        enabled: true,
        truecolor: true,
    };
    assert_eq!(
        true_color.paint("x", Color::Rgb(1, 2, 3)),
        "\x1b[38;2;1;2;3mx\x1b[0m"
    );
}

#[test]
fn visible_width_skips_escape_codes() {
    assert_eq!(style::visible_width("\x1b[1;33mOS\x1b[0m:"), 3);
    assert_eq!(style::visible_width("██▍░"), 4);
}

#[test]
fn truecolor_detection() {
    assert!(style::detect_truecolor(Some("truecolor"), None));
    assert!(style::detect_truecolor(Some("24bit"), None));
    assert!(style::detect_truecolor(None, Some("iTerm.app")));
    assert!(!style::detect_truecolor(None, Some("Apple_Terminal")));
    assert!(!style::detect_truecolor(Some("yes"), None));
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cli_flags() {
    let cli = cli::parse(args(&[
        "-a",
        "--json",
        "--color=never",
        "-p",
        "5",
        "-m",
        "os, cpu,memory",
    ]))
    .unwrap();
    assert!(cli.all && cli.json);
    assert_eq!(cli.color, ColorWhen::Never);
    assert_eq!(cli.padding, Some(5));
    assert_eq!(cli.modules, Some(args(&["os", "cpu", "memory"])));

    let cli = cli::parse(args(&[
        "--no-logo",
        "--config-file",
        "/tmp/x.toml",
        "--reset-config",
    ]))
    .unwrap();
    assert_eq!(cli.logo.as_deref(), Some("none"));
    assert_eq!(
        cli.config.as_deref(),
        Some(std::path::Path::new("/tmp/x.toml"))
    );
    assert!(cli.init_config);

    assert!(cli::parse(args(&["--bogus"])).is_err());
    assert!(cli::parse(args(&["--padding"])).is_err());
    assert!(cli::parse(args(&["--padding", "x"])).is_err());
    assert!(cli::parse(args(&["--color", "sometimes"])).is_err());
}

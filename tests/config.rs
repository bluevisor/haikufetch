#![allow(clippy::unwrap_used, clippy::expect_used)]

use haikufetch::{
    config::{self, DEFAULT_MODULES},
    modules,
};

#[test]
fn an_empty_file_means_defaults() {
    let c = config::parse("");
    assert_eq!(c.modules, DEFAULT_MODULES);
    assert!(c.warnings.is_empty());
    assert_eq!(c.padding, 3);
}

#[test]
fn the_template_is_the_defaults_and_has_no_warnings() {
    let c = config::parse(config::TEMPLATE);
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(c.modules, DEFAULT_MODULES);
    assert!(c.bars && c.cpu_frequency && c.refresh_rate && c.battery_adapter && c.tailscale_name);
    assert_eq!(c.disks, ["/"]);
}

#[test]
fn every_default_module_exists() {
    for name in DEFAULT_MODULES {
        assert!(modules::find(name).is_some(), "{name}");
    }
}

#[test]
fn module_names_and_aliases_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for m in modules::MODULES {
        for name in std::iter::once(&m.name).chain(m.aliases) {
            assert!(seen.insert(*name), "{name} used twice");
        }
    }
    assert_eq!(modules::find("RAM").map(|m| m.name), Some("memory"));
    assert_eq!(modules::find("power-draw").map(|m| m.name), Some("power"));
}

#[test]
fn settings_are_read() {
    let c = config::parse(
        r##"
modules = ["os", "cpu"]
[logo]
name = "arch"
color = "#ff8800"
padding = 1
[style]
key_color = "red"
bars = false
bar_width = 20
[cpu]
frequency = false
[disk]
paths = ["/", "/home"]
[keys]
memory = "Memory"
"##,
    );
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(c.modules, ["os", "cpu"]);
    assert_eq!(
        (c.logo.as_str(), c.logo_color.as_str(), c.padding),
        ("arch", "#ff8800", 1)
    );
    assert_eq!(c.key_color, "red");
    assert!(!c.bars && !c.cpu_frequency);
    assert_eq!(c.bar_width, 20);
    assert_eq!(c.disks, ["/", "/home"]);
    assert_eq!(c.keys.get("memory").map(String::as_str), Some("Memory"));
}

#[test]
fn a_bad_value_is_one_warning_and_the_rest_still_applies() {
    let c = config::parse(
        r#"
modules = "os"
typo = 1
[logo]
padding = "wide"
color = "red"
[style]
bars = "yes"
key_color = false
"#,
    );
    assert_eq!(c.warnings.len(), 4, "{:?}", c.warnings);
    assert_eq!(c.modules, DEFAULT_MODULES);
    assert_eq!(c.padding, 3);
    assert_eq!(c.logo_color, "red");
    assert!(c.bars);
    assert_eq!(c.key_color, "none");
}

#[test]
fn invalid_toml_is_a_warning_not_a_crash() {
    let c = config::parse("[logo\nname = ");
    assert_eq!(c.warnings.len(), 1);
    assert_eq!(c.modules, DEFAULT_MODULES);
}

#[test]
fn old_rustfetch_configs_keep_working() {
    let c = config::parse(
        r#"
[display]
identifier = true
os = true
kernel = true
uptime = true
cpu = true
cpu_frequency = true
gpu = true
screen = true
resolution = true
refresh_rate = false
ram = true
swap = false
disk = true
usage_bars = false
key_color = "auto"
tailscale_ip = true
tailscale_host = false
battery = true
power_draw = false

[logo]
name = "seen"
color = "rainbow"
"#,
    );
    assert!(c.warnings.is_empty(), "{:?}", c.warnings);
    assert_eq!(
        c.modules,
        [
            "title",
            "separator",
            "os",
            "kernel",
            "uptime",
            "shell",
            "screen",
            "cpu",
            "gpu",
            "memory",
            "disk",
            "local_ip",
            "ipv6",
            "wifi",
            "tailscale",
            "battery"
        ]
    );
    assert!(c.cpu_frequency && !c.refresh_rate && !c.bars && !c.tailscale_name);
    assert_eq!(
        (c.logo.as_str(), c.logo_color.as_str()),
        ("seen", "rainbow")
    );
}

#[test]
fn old_configs_without_newer_keys_default_them_on() {
    let c = config::parse("[display]\nos = true\n");
    assert!(
        c.modules.contains(&String::from("wifi")) && c.modules.contains(&String::from("shell"))
    );
    // the 0.x default
    assert!(!c.cpu_frequency);
    assert!(!c.modules.contains(&String::from("power")));
}

#[test]
fn logo_colors_are_parsed_and_bad_ones_reported() {
    let c = config::parse("[logo]\ncolors = [\"#a7e2c0\", \"cyan\", \"mauve\"]\n");
    assert_eq!(
        c.logo_colors,
        [
            haikufetch::style::Color::Rgb(0xa7, 0xe2, 0xc0),
            haikufetch::style::Color::Ansi(6)
        ]
    );
    assert_eq!(c.warnings.len(), 1, "{:?}", c.warnings);
}

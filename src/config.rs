//! The config file. It is read as a plain TOML table, key by key: a misspelled key or a wrong type
//! is reported once on stderr and that one setting keeps its default, the rest of the file still
//! applies. A missing file just means the defaults; nothing is written unless --init-config asks.
//!
//! Config files from rustfetch 0.x (a [display] table of on/off switches) are still understood
//! and translated to the module list they describe.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use toml::{Table, Value};

use crate::paths;

/// The modules shown when the config doesn't list its own
pub const DEFAULT_MODULES: &[&str] = &[
    "title",
    "separator",
    "os",
    "host",
    "kernel",
    "uptime",
    "packages",
    "shell",
    "terminal",
    "screen",
    "cpu",
    "gpu",
    "memory",
    "swap",
    "disk",
    "local_ip",
    "ipv6",
    "wifi",
    "tailscale",
    "battery",
    "break",
    "colors",
];

#[derive(Debug, Clone)]
pub struct Config {
    pub modules: Vec<String>,
    /// "auto", a built-in logo name, "none", or a path to a text file
    pub logo: String,
    /// "default", "rainbow" or a color
    pub logo_color: String,
    /// The colors of a logo's $1, $2, ... markers, replacing its own (custom logos need these)
    pub logo_colors: Vec<crate::style::Color>,
    pub padding: usize,
    /// "auto", "none" or a color
    pub key_color: String,
    pub bars: bool,
    pub bar_width: usize,
    pub cpu_frequency: bool,
    pub gpu_frequency: bool,
    pub refresh_rate: bool,
    pub battery_adapter: bool,
    pub disks: Vec<String>,
    pub tailscale_name: bool,
    /// Custom labels, module name to key text
    pub keys: HashMap<String, String>,
    /// Problems found while reading the file, printed once on stderr
    pub warnings: Vec<String>,
    /// Where the settings came from, None for the built-in defaults
    pub source: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            modules: DEFAULT_MODULES.iter().map(|m| m.to_string()).collect(),
            logo: String::from("auto"),
            logo_color: String::from("default"),
            logo_colors: Vec::new(),
            padding: 3,
            key_color: String::from("auto"),
            bars: true,
            bar_width: 10,
            cpu_frequency: true,
            gpu_frequency: true,
            refresh_rate: true,
            battery_adapter: true,
            disks: vec![String::from("/")],
            tailscale_name: true,
            keys: HashMap::new(),
            warnings: Vec::new(),
            source: None,
        }
    }
}

/// The default config file, written by --init-config. Every value here is the built-in default
pub const TEMPLATE: &str = r##"# haikufetch config
# Every setting is optional: delete a line and it goes back to its default.
# haikufetch --list-modules shows what each module does.

# The lines to show, top to bottom. "break" is an empty line.
modules = [
    "title", "separator",
    "os", "host", "kernel", "uptime", "packages", "shell", "terminal",
    "screen", "cpu", "gpu", "memory", "swap", "disk",
    "local_ip", "ipv6", "wifi", "tailscale", "battery",
    "break", "colors",
]

[logo]
# "auto" draws your system's logo, "none" hides it. Any name from --list-logos works:
# built-in, or your own art saved as ~/.config/haikufetch/logos/<name>.txt. A path works too.
name = "auto"
# Colors for the art's $1, $2, ... markers, in place of the logo's own. Your own art needs these
# to be anything but white, e.g. colors = ["#a7e2c0", "#5dc6b2", "#00a7a2"]
# colors = []
# "default" keeps the logo's own colors. "rainbow" draws the six-color Apple logo.
# Anything else paints the whole logo: a name (red, bright blue, ...) or a hex code (#ff8800).
color = "default"
# Spaces between the logo and the text.
padding = 3

[style]
# Color of each line's label. "auto" follows the logo, "none" keeps plain bold labels,
# or a name or hex code.
key_color = "auto"
# Draw usage bars after memory, swap and disk figures.
bars = true
bar_width = 10

[cpu]
frequency = true

[gpu]
frequency = true

[screen]
refresh_rate = true

[disk]
# One line per path, for the filesystem holding it.
paths = ["/"]

[tailscale]
# Also show the machine's Tailscale name (asks Tailscale's DNS, ~1 ms).
name = true

[battery]
# Name the power adapter while one is plugged in (macOS).
adapter = true

# Rename any line's label, e.g. memory = "RAM".
[keys]
"##;

/// Finds the config file: --config, then the XDG location, then (macOS) where rustfetch 0.x kept it
pub fn default_path() -> Option<PathBuf> {
    let primary = paths::config_dir()?.join("haikufetch/config.toml");
    if primary.exists() {
        return Some(primary);
    }
    paths::legacy_config_dirs()
        .into_iter()
        .map(|dir| dir.join("rustfetch/config.toml"))
        .find(|path| path.exists())
        .or(Some(primary))
}

/// Loads the config, from `path` or the default location. Never fails: problems become warnings
pub fn load(path: Option<&Path>) -> Config {
    let explicit = path.is_some();
    let Some(path) = path.map(Path::to_path_buf).or_else(default_path) else {
        return Config::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(text) => {
            let mut config = parse(&text);
            config.source = Some(path);
            config
        },
        Err(error) => {
            let mut config = Config::default();
            if explicit {
                config
                    .warnings
                    .push(format!("can't read {}: {error}", path.display()));
            }
            config
        },
    }
}

/// Parses a config file's text
pub fn parse(text: &str) -> Config {
    let mut config = Config::default();
    let table: Table = match text.parse() {
        Ok(table) => table,
        Err(error) => {
            config
                .warnings
                .push(format!("config is not valid TOML, using defaults: {error}"));
            return config;
        },
    };

    if table.get("display").is_some_and(Value::is_table) {
        apply_legacy(&table, &mut config);
        return config;
    }

    let mut reader = Reader {
        warnings: &mut config.warnings,
    };
    for (key, value) in &table {
        match (key.as_str(), value) {
            ("modules", value) => {
                if let Some(list) = reader.strings("modules", value) {
                    config.modules = list;
                }
            },
            ("logo", Value::Table(logo)) => {
                for (key, value) in logo {
                    let path = format!("logo.{key}");
                    match key.as_str() {
                        "name" => reader.string(&path, value).map(|v| config.logo = v),
                        "color" => reader.string(&path, value).map(|v| config.logo_color = v),
                        "padding" => reader.number(&path, value).map(|v| config.padding = v),
                        "colors" => reader.strings(&path, value).map(|list| {
                            for item in list {
                                match crate::style::parse_color(&item) {
                                    Some(color) => config.logo_colors.push(color),
                                    None => reader.wrong(&path, "a list of colors"),
                                }
                            }
                        }),
                        _ => reader.unknown(&path),
                    };
                }
            },
            ("style", Value::Table(style)) => {
                for (key, value) in style {
                    let path = format!("style.{key}");
                    match key.as_str() {
                        "key_color" => reader.color(&path, value).map(|v| config.key_color = v),
                        "bars" => reader.bool(&path, value).map(|v| config.bars = v),
                        "bar_width" => reader
                            .number(&path, value)
                            .map(|v| config.bar_width = v.clamp(1, 50)),
                        _ => reader.unknown(&path),
                    };
                }
            },
            ("cpu", Value::Table(t)) => {
                reader.switch(t, "cpu", "frequency", &mut config.cpu_frequency)
            },
            ("gpu", Value::Table(t)) => {
                reader.switch(t, "gpu", "frequency", &mut config.gpu_frequency)
            },
            ("screen", Value::Table(t)) => {
                reader.switch(t, "screen", "refresh_rate", &mut config.refresh_rate)
            },
            ("battery", Value::Table(t)) => {
                reader.switch(t, "battery", "adapter", &mut config.battery_adapter)
            },
            ("tailscale", Value::Table(t)) => {
                reader.switch(t, "tailscale", "name", &mut config.tailscale_name)
            },
            ("disk", Value::Table(disk)) => {
                for (key, value) in disk {
                    match key.as_str() {
                        "paths" => {
                            if let Some(list) = reader.strings("disk.paths", value) {
                                config.disks = list;
                            }
                        },
                        _ => {
                            reader.unknown(&format!("disk.{key}"));
                        },
                    }
                }
            },
            ("keys", Value::Table(keys)) => {
                for (key, value) in keys {
                    if let Some(label) = reader.string(&format!("keys.{key}"), value) {
                        config.keys.insert(key.clone(), label);
                    }
                }
            },
            (key, _)
                if [
                    "logo",
                    "style",
                    "cpu",
                    "gpu",
                    "screen",
                    "battery",
                    "tailscale",
                    "disk",
                    "keys",
                ]
                .contains(&key) =>
            {
                reader.warnings.push(format!("[{key}] should be a table"));
            },
            (key, _) => {
                reader.unknown(key);
            },
        }
    }
    config
}

/// Reads typed values out of the table, recording a warning for each one that doesn't fit
struct Reader<'a> {
    warnings: &'a mut Vec<String>,
}

impl Reader<'_> {
    fn unknown(&mut self, path: &str) -> Option<()> {
        self.warnings
            .push(format!("unknown setting {path:?}, ignored"));
        None
    }

    fn wrong(&mut self, path: &str, expected: &str) {
        self.warnings
            .push(format!("{path} should be {expected}, ignored"));
    }

    fn string(&mut self, path: &str, value: &Value) -> Option<String> {
        match value {
            Value::String(s) => Some(s.clone()),
            _ => {
                self.wrong(path, "a string");
                None
            },
        }
    }

    /// A color setting also takes `false`, meaning "none"
    fn color(&mut self, path: &str, value: &Value) -> Option<String> {
        match value {
            Value::Boolean(false) => Some(String::from("none")),
            Value::Boolean(true) => Some(String::from("auto")),
            _ => self.string(path, value),
        }
    }

    fn bool(&mut self, path: &str, value: &Value) -> Option<bool> {
        match value {
            Value::Boolean(b) => Some(*b),
            _ => {
                self.wrong(path, "true or false");
                None
            },
        }
    }

    fn number(&mut self, path: &str, value: &Value) -> Option<usize> {
        match value {
            Value::Integer(n) if *n >= 0 => Some((*n).min(255) as usize),
            _ => {
                self.wrong(path, "a positive number");
                None
            },
        }
    }

    fn strings(&mut self, path: &str, value: &Value) -> Option<Vec<String>> {
        let list = match value {
            Value::Array(items) => items
                .iter()
                .map(|v| v.as_str().map(str::to_string))
                .collect(),
            _ => None,
        };
        if list.is_none() {
            self.wrong(path, "a list of strings");
        }
        list
    }

    /// A table with a single on/off setting, like [cpu] frequency
    fn switch(&mut self, table: &Table, section: &str, name: &str, target: &mut bool) {
        for (key, value) in table {
            let path = format!("{section}.{key}");
            if key == name {
                if let Some(v) = self.bool(&path, value) {
                    *target = v;
                }
            } else {
                self.unknown(&path);
            }
        }
    }
}

/// Translates a rustfetch 0.x config ([display] switches, [logo] color and name) into this one.
/// The module order is the one 0.x printed in
fn apply_legacy(table: &Table, config: &mut Config) {
    let display = table.get("display").and_then(Value::as_table);
    let on = |key: &str, default: bool| {
        display
            .and_then(|d| d.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(default)
    };

    let mut modules = Vec::new();
    let mut add = |enabled: bool, names: &[&str]| {
        if enabled {
            modules.extend(names.iter().map(|n| n.to_string()));
        }
    };
    add(on("identifier", true), &["title", "separator"]);
    add(on("os", true), &["os"]);
    add(on("kernel", true), &["kernel"]);
    add(on("uptime", true), &["uptime"]);
    add(on("shell", true), &["shell"]);
    add(
        on("screen", true) && (on("resolution", true) || on("refresh_rate", true)),
        &["screen"],
    );
    add(on("cpu", true), &["cpu"]);
    add(on("gpu", true), &["gpu"]);
    add(on("ram", true), &["memory"]);
    add(on("disk", true), &["disk"]);
    add(on("swap", true), &["swap"]);
    add(on("ipv4", true), &["local_ip"]);
    add(on("ipv6", true), &["ipv6"]);
    add(on("wifi", true), &["wifi"]);
    let tailscale_name = display
        .and_then(|d| d.get("tailscale_name").or_else(|| d.get("tailscale_host")))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    add(on("tailscale_ip", true) || tailscale_name, &["tailscale"]);
    add(on("battery", true), &["battery"]);
    add(on("power_draw", false), &["power"]);
    config.modules = modules;

    config.cpu_frequency = on("cpu_frequency", false);
    config.refresh_rate = on("refresh_rate", true);
    config.bars = on("usage_bars", true);
    config.battery_adapter = on("ac_adapter", true);
    config.tailscale_name = tailscale_name;
    if let Some(value) = display.and_then(|d| d.get("key_color")) {
        config.key_color = match value {
            Value::String(s) => s.clone(),
            Value::Boolean(false) => String::from("none"),
            _ => String::from("auto"),
        };
    }

    if let Some(logo) = table.get("logo").and_then(Value::as_table) {
        if let Some(color) = logo.get("color").and_then(Value::as_str) {
            config.logo_color = color.to_string();
        }
        if let Some(name) = logo.get("name").and_then(Value::as_str) {
            config.logo = name.to_string();
        }
    }
}

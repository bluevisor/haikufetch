//! The modules: one per kind of line. Each turns platform facts into entries, a label and a
//! colored value for the terminal plus the same facts as JSON. `run` collects every requested
//! module, the slow ones (anything that runs a program, asks the network or loads a framework) on
//! their own threads so their waits overlap.

use std::{
    net::IpAddr,
    time::{Duration, Instant},
};

use crate::{
    config::Config,
    format,
    info::*,
    platform, shell,
    style::{Accent, Painter},
    tailscale, unix,
};

/// One printed line. `key` empty means the value is printed on its own (title, separator, colors)
pub struct Entry {
    pub key: String,
    pub value: String,
    /// The facts behind the line as a JSON value
    pub json: String,
}

pub struct Ctx<'a> {
    pub config: &'a Config,
    pub painter: Painter,
}

pub struct Module {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub about: &'static str,
    /// Worth its own thread: waits on a program, the network or a framework load
    pub slow: bool,
    pub run: fn(&Ctx) -> Vec<Entry>,
}

pub const MODULES: &[Module] = &[
    Module {
        name: "title",
        aliases: &["identifier"],
        about: "user@host",
        slow: false,
        run: title,
    },
    Module {
        name: "separator",
        aliases: &[],
        about: "a line under the title",
        slow: false,
        run: separator,
    },
    Module {
        name: "os",
        aliases: &[],
        about: "operating system, version and architecture",
        slow: false,
        run: os,
    },
    Module {
        name: "host",
        aliases: &["model"],
        about: "the machine's model",
        slow: false,
        run: host,
    },
    Module {
        name: "kernel",
        aliases: &[],
        about: "kernel name and release",
        slow: false,
        run: kernel,
    },
    Module {
        name: "uptime",
        aliases: &[],
        about: "time since boot",
        slow: false,
        run: uptime,
    },
    Module {
        name: "packages",
        aliases: &[],
        about: "installed packages per package manager",
        slow: true,
        run: packages,
    },
    Module {
        name: "shell",
        aliases: &[],
        about: "the shell haikufetch runs in, and its version",
        slow: true,
        run: shell,
    },
    Module {
        name: "terminal",
        aliases: &[],
        about: "the terminal emulator",
        slow: false,
        run: terminal,
    },
    Module {
        name: "screen",
        aliases: &["display", "resolution"],
        about: "each display's resolution and refresh rate",
        slow: true,
        run: screen,
    },
    Module {
        name: "cpu",
        aliases: &[],
        about: "processor, core counts and top frequency",
        slow: false,
        run: cpu,
    },
    Module {
        name: "gpu",
        aliases: &[],
        about: "graphics processors",
        slow: true,
        run: gpu,
    },
    Module {
        name: "memory",
        aliases: &["ram"],
        about: "RAM in use",
        slow: false,
        run: memory,
    },
    Module {
        name: "swap",
        aliases: &[],
        about: "swap in use",
        slow: false,
        run: swap,
    },
    Module {
        name: "disk",
        aliases: &[],
        about: "space used on each [disk] path's filesystem",
        slow: false,
        run: disk,
    },
    Module {
        name: "local_ip",
        aliases: &["ipv4", "ip"],
        about: "IPv4 address of the main network interface",
        slow: false,
        run: local_ip,
    },
    Module {
        name: "ipv6",
        aliases: &[],
        about: "global IPv6 address of the main interface",
        slow: false,
        run: ipv6,
    },
    Module {
        name: "wifi",
        aliases: &["ssid"],
        about: "the Wi-Fi network's name",
        slow: true,
        run: wifi,
    },
    Module {
        name: "tailscale",
        aliases: &["tailscale_ip", "tailscale_name"],
        about: "Tailscale address and machine name",
        slow: true,
        run: tailscale,
    },
    Module {
        name: "battery",
        aliases: &[],
        about: "charge, state and power adapter",
        slow: true,
        run: battery,
    },
    Module {
        name: "power",
        aliases: &["power_draw"],
        about: "power drawn from the battery (Linux)",
        slow: false,
        run: power,
    },
    Module {
        name: "locale",
        aliases: &[],
        about: "the language setting (LANG)",
        slow: false,
        run: locale,
    },
    Module {
        name: "colors",
        aliases: &["palette"],
        about: "the terminal's 16 colors",
        slow: false,
        run: colors,
    },
    Module {
        name: "break",
        aliases: &["empty"],
        about: "an empty line",
        slow: false,
        run: blank,
    },
];

pub fn find(name: &str) -> Option<&'static Module> {
    let name = name.trim().to_ascii_lowercase().replace('-', "_");
    MODULES
        .iter()
        .find(|m| m.name == name || m.aliases.contains(&name.as_str()))
}

/// What one module produced, and how long it took
pub struct Output {
    pub module: &'static Module,
    pub entries: Vec<Entry>,
    pub took: Duration,
}

/// Runs the modules, slow ones on scoped threads, and returns their output in the order asked
pub fn run(modules: &[&'static Module], ctx: &Ctx) -> Vec<Output> {
    let timed = |module: &'static Module| {
        let start = Instant::now();
        let entries = (module.run)(ctx);
        Output {
            module,
            entries,
            took: start.elapsed(),
        }
    };

    std::thread::scope(|scope| {
        let handles: Vec<_> = modules
            .iter()
            .map(|&module| module.slow.then(|| scope.spawn(move || timed(module))))
            .collect();
        modules
            .iter()
            .zip(handles)
            .map(|(&module, handle)| match handle {
                // A module that panicked shows nothing rather than taking the rest down
                Some(handle) => handle.join().unwrap_or(Output {
                    module,
                    entries: Vec::new(),
                    took: Duration::ZERO,
                }),
                None => timed(module),
            })
            .collect()
    })
}

// ---- helpers ---------------------------------------------------------------------------------

fn entry(ctx: &Ctx, module: &str, default_key: &str, value: String, json: String) -> Entry {
    let key = ctx
        .config
        .keys
        .get(module)
        .cloned()
        .unwrap_or_else(|| default_key.to_string());
    Entry { key, value, json }
}

fn plain(value: String, json: String) -> Entry {
    Entry {
        key: String::new(),
        value,
        json,
    }
}

fn json_object(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("{}:{}", format::json_string(k), v))
        .collect();
    format!("{{{}}}", body.join(","))
}

fn js(value: &str) -> String {
    format::json_string(value)
}

fn jopt(value: Option<&str>) -> String {
    value.map(js).unwrap_or_else(|| String::from("null"))
}

fn jnum<T: ToString>(value: Option<T>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| String::from("null"))
}

/// Low usage is green, high is red
fn usage_accent(percent: u8) -> Accent {
    match percent {
        0..=39 => Accent::Green,
        40..=79 => Accent::Yellow,
        _ => Accent::Red,
    }
}

/// "50.2 GiB / 64 GiB ███████▊░░ 78%", or "(78%)" in place of the bar with bars off
fn usage_value(ctx: &Ctx, usage: Usage) -> String {
    let p = &ctx.painter;
    let figures = format!(
        "{} / {}",
        format::size(usage.used),
        format::size(usage.total)
    );
    let Some(percent) = format::percent(usage.used, usage.total) else {
        return figures;
    };
    let color = p.accent(usage_accent(percent));
    let label = p.paint(&format!("{percent}%"), color);
    if !ctx.config.bars {
        return format!("{figures} ({label})");
    }
    let (filled, empty) = format::bar(percent, ctx.config.bar_width);
    let mut bar = String::new();
    if !filled.is_empty() {
        bar.push_str(&p.paint(&filled, color));
    }
    if !empty.is_empty() {
        bar.push_str(&p.paint(&empty, p.track()));
    }
    format!("{figures} {bar} {label}")
}

fn usage_json(usage: Usage) -> String {
    json_object(&[
        ("used", usage.used.to_string()),
        ("total", usage.total.to_string()),
        ("percent", jnum(format::percent(usage.used, usage.total))),
    ])
}

// ---- modules ---------------------------------------------------------------------------------

fn user_at_host() -> (String, String) {
    let user = unix::username().unwrap_or_else(|| String::from("user"));
    // macOS reports the Bonjour name (Johns-MacBook-Pro.local), the suffix is noise
    let host = unix::hostname().unwrap_or_default();
    let host = host.strip_suffix(".local").unwrap_or(&host).to_string();
    (user, host)
}

fn title(ctx: &Ctx) -> Vec<Entry> {
    let (user, host) = user_at_host();
    let p = &ctx.painter;
    let key_color = crate::render::key_color();
    let part = |text: &str| match key_color {
        Some(color) => {
            let mut s = String::new();
            p.push_bold(&mut s);
            p.push_fg(&mut s, color);
            s.push_str(text);
            p.push_reset(&mut s);
            s
        },
        None => p.bold(text),
    };
    let value = format!("{}@{}", part(&user), part(&host));
    vec![plain(
        value,
        json_object(&[("user", js(&user)), ("host", js(&host))]),
    )]
}

fn separator(_: &Ctx) -> Vec<Entry> {
    let (user, host) = user_at_host();
    let width = user.chars().count() + 1 + host.chars().count();
    vec![plain("-".repeat(width), String::from("null"))]
}

fn os(ctx: &Ctx) -> Vec<Entry> {
    let os = platform::os();
    let arch = std::env::consts::ARCH;
    let value = format!("{} ({arch})", os.pretty);
    let json = json_object(&[
        ("id", js(&os.id)),
        ("name", js(&os.pretty)),
        ("version", jopt(os.version.as_deref())),
        ("codename", jopt(os.codename.as_deref())),
        ("build", jopt(os.build.as_deref())),
        ("arch", js(arch)),
    ]);
    vec![entry(ctx, "os", "OS", value, json)]
}

fn host(ctx: &Ctx) -> Vec<Entry> {
    platform::host_model()
        .map(|model| entry(ctx, "host", "Host", model.clone(), js(&model)))
        .into_iter()
        .collect()
}

fn kernel(ctx: &Ctx) -> Vec<Entry> {
    let Some((name, release)) = unix::kernel() else {
        return Vec::new();
    };
    let value = format!("{name} {release}");
    let json = json_object(&[("name", js(&name)), ("release", js(&release))]);
    vec![entry(ctx, "kernel", "Kernel", value, json)]
}

fn uptime(ctx: &Ctx) -> Vec<Entry> {
    platform::uptime_seconds()
        .map(|s| entry(ctx, "uptime", "Uptime", format::duration(s), s.to_string()))
        .into_iter()
        .collect()
}

fn packages(ctx: &Ctx) -> Vec<Entry> {
    let found: Vec<Packages> = platform::packages()
        .into_iter()
        .filter(|p| p.count > 0)
        .collect();
    if found.is_empty() {
        return Vec::new();
    }
    let value = found
        .iter()
        .map(|p| format!("{} ({})", p.count, p.manager))
        .collect::<Vec<_>>()
        .join(", ");
    let json = json_object(
        &found
            .iter()
            .map(|p| (p.manager, p.count.to_string()))
            .collect::<Vec<_>>(),
    );
    vec![entry(ctx, "packages", "Packages", value, json)]
}

fn shell(ctx: &Ctx) -> Vec<Entry> {
    let Some(shell) = shell::detect() else {
        return Vec::new();
    };
    let value = match &shell.version {
        Some(version) => format!("{} {version}", shell.name),
        None => shell.name.clone(),
    };
    let json = json_object(&[
        ("name", js(&shell.name)),
        ("version", jopt(shell.version.as_deref())),
    ]);
    vec![entry(ctx, "shell", "Shell", value, json)]
}

fn terminal(ctx: &Ctx) -> Vec<Entry> {
    unix::terminal()
        .map(|name| entry(ctx, "terminal", "Terminal", name.clone(), js(&name)))
        .into_iter()
        .collect()
}

fn screen(ctx: &Ctx) -> Vec<Entry> {
    let mut screens = platform::screens();
    // stable, so only the primary display moves to the front
    screens.sort_by_key(|s| !s.primary);
    let count = screens.len();
    screens
        .iter()
        .enumerate()
        .map(|(index, s)| {
            let key = match (count, s.name.is_empty()) {
                (0 | 1, _) => String::from("Screen"),
                (_, true) => format!("Screen {}", index + 1),
                (_, false) => format!("Screen ({})", s.name),
            };
            let mut value = format!("{}x{}", s.width, s.height);
            if ctx.config.refresh_rate
                && let Some(hz) = s.refresh_hz
            {
                value.push_str(&format!(" @ {hz}Hz"));
            }
            let json = json_object(&[
                ("name", js(&s.name)),
                ("width", s.width.to_string()),
                ("height", s.height.to_string()),
                ("refresh_hz", jnum(s.refresh_hz)),
                ("primary", s.primary.to_string()),
            ]);
            let key = ctx
                .config
                .keys
                .get("screen")
                .cloned()
                .filter(|_| count <= 1)
                .unwrap_or(key);
            Entry { key, value, json }
        })
        .collect()
}

fn cpu(ctx: &Ctx) -> Vec<Entry> {
    let Some(cpu) = platform::cpu() else {
        return Vec::new();
    };
    let mut value = cpu.name.clone();
    if let Some(cores) = format::cores(&cpu.cores) {
        value.push_str(&format!(" ({cores})"));
    }
    if ctx.config.cpu_frequency
        && let Some(freq) = format::frequency(cpu.max_mhz)
    {
        value.push_str(&format!(" @ {freq}"));
    }
    let cores = format!(
        "[{}]",
        cpu.cores
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    let json = json_object(&[
        ("name", js(&cpu.name)),
        ("cores", cores),
        ("max_mhz", jnum((cpu.max_mhz > 0).then_some(cpu.max_mhz))),
    ]);
    vec![entry(ctx, "cpu", "CPU", value, json)]
}

fn gpu(ctx: &Ctx) -> Vec<Entry> {
    let gpus = platform::gpus();
    let count = gpus.len();
    gpus.iter()
        .enumerate()
        .map(|(index, gpu)| {
            let mut value = gpu.name.clone();
            if let Some(cores) = gpu.cores {
                value.push_str(&format!(" ({cores} cores)"));
            }
            if ctx.config.gpu_frequency
                && let Some(freq) = format::frequency(gpu.max_mhz)
            {
                value.push_str(&format!(" @ {freq}"));
            }
            let json = json_object(&[
                ("name", js(&gpu.name)),
                ("cores", jnum(gpu.cores)),
                ("max_mhz", jnum((gpu.max_mhz > 0).then_some(gpu.max_mhz))),
            ]);
            let default = if count > 1 {
                format!("GPU {}", index + 1)
            } else {
                String::from("GPU")
            };
            entry(ctx, "gpu", &default, value, json)
        })
        .collect()
}

fn memory(ctx: &Ctx) -> Vec<Entry> {
    platform::memory()
        .map(|u| entry(ctx, "memory", "RAM", usage_value(ctx, u), usage_json(u)))
        .into_iter()
        .collect()
}

fn swap(ctx: &Ctx) -> Vec<Entry> {
    let Some(usage) = platform::swap() else {
        return Vec::new();
    };
    if usage.total == 0 {
        return vec![entry(
            ctx,
            "swap",
            "Swap",
            String::from("Disabled"),
            usage_json(usage),
        )];
    }
    vec![entry(
        ctx,
        "swap",
        "Swap",
        usage_value(ctx, usage),
        usage_json(usage),
    )]
}

fn disk(ctx: &Ctx) -> Vec<Entry> {
    ctx.config
        .disks
        .iter()
        .filter_map(|path| {
            let real = platform::disk_path(path);
            let usage = unix::filesystem_usage(&real)?;
            let path = &real;
            let key = ctx
                .config
                .keys
                .get("disk")
                .map(|k| k.replace("{path}", path))
                .unwrap_or_else(|| format!("Disk ({path})"));
            let json = format!("{{\"path\":{},{}", js(path), &usage_json(usage)[1..]);
            Some(Entry {
                key,
                value: usage_value(ctx, usage),
                json,
            })
        })
        .collect()
}

fn address_json(a: &InterfaceAddress) -> String {
    json_object(&[
        ("interface", js(&a.interface)),
        ("address", js(&a.address.to_string())),
        ("prefix", a.prefix.to_string()),
    ])
}

/// Haiku names interfaces by device path: /dev/net/ipro1000/0 reads as ipro1000/0
fn short_interface(name: &str) -> &str {
    name.strip_prefix("/dev/net/").unwrap_or(name)
}

fn local_ip(ctx: &Ctx) -> Vec<Entry> {
    let Some(a) = crate::net::local_address(false) else {
        return Vec::new();
    };
    let key = format!("Local IPv4 ({})", short_interface(&a.interface));
    let value = format!("{}/{}", a.address, a.prefix);
    vec![entry(ctx, "local_ip", &key, value, address_json(&a))]
}

fn ipv6(ctx: &Ctx) -> Vec<Entry> {
    let Some(a) = crate::net::local_address(true) else {
        return Vec::new();
    };
    let key = format!("Local IPv6 ({})", short_interface(&a.interface));
    let value = format!("{}/{}", a.address, a.prefix);
    vec![entry(ctx, "ipv6", &key, value, address_json(&a))]
}

fn wifi(ctx: &Ctx) -> Vec<Entry> {
    platform::wifi_ssid()
        .map(|ssid| entry(ctx, "wifi", "Wi-Fi", ssid.clone(), js(&ssid)))
        .into_iter()
        .collect()
}

fn tailscale(ctx: &Ctx) -> Vec<Entry> {
    let Some(address) = tailscale::address() else {
        return Vec::new();
    };
    let name = ctx
        .config
        .tailscale_name
        .then(|| tailscale::machine_name(address))
        .flatten();
    let value = match &name {
        Some(name) => format!("{address} ({name})"),
        None => address.to_string(),
    };
    let json = json_object(&[
        ("address", js(&IpAddr::V4(address).to_string())),
        ("name", jopt(name.as_deref())),
    ]);
    vec![entry(ctx, "tailscale", "Tailscale", value, json)]
}

fn battery(ctx: &Ctx) -> Vec<Entry> {
    let Some(b) = platform::battery(ctx.config.battery_adapter) else {
        return Vec::new();
    };
    let p = &ctx.painter;
    // charge reads the other way round from usage: low is red
    let accent = match b.percent {
        0..=29 => Accent::Red,
        30..=69 => Accent::Yellow,
        _ => Accent::Green,
    };
    let percent = p.paint(&format!("{}%", b.percent), p.accent(accent));
    // an adapter left in AdapterDetails after unplugging isn't shown on battery power
    let adapter = b
        .adapter
        .as_deref()
        .filter(|a| !a.is_empty() && b.charge != Charge::Discharging);
    let status = match adapter {
        Some(adapter) if b.charge == Charge::NotCharging => adapter.to_string(),
        Some(adapter) => format!("{}, {adapter}", b.charge.label()),
        None => b.charge.label().to_string(),
    };
    let json = json_object(&[
        ("percent", b.percent.to_string()),
        ("state", js(b.charge.label())),
        ("adapter", jopt(adapter)),
    ]);
    vec![entry(
        ctx,
        "battery",
        "Battery",
        format!("{percent} ({status})"),
        json,
    )]
}

fn power(ctx: &Ctx) -> Vec<Entry> {
    platform::power_draw_watts()
        .filter(|w| *w > 0.0)
        .map(|w| {
            entry(
                ctx,
                "power",
                "Power Draw",
                format!("{w:.1} W"),
                format!("{w:.2}"),
            )
        })
        .into_iter()
        .collect()
}

fn locale(ctx: &Ctx) -> Vec<Entry> {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
        .map(|l| entry(ctx, "locale", "Locale", l.clone(), js(&l)))
        .into_iter()
        .collect()
}

fn colors(ctx: &Ctx) -> Vec<Entry> {
    if !ctx.painter.enabled {
        return Vec::new();
    }
    let row = |base: u8| {
        (0..8)
            .map(|i| format!("\x1b[{}m   ", base + i))
            .collect::<String>()
            + "\x1b[0m"
    };
    vec![
        plain(row(40), String::from("null")),
        plain(row(100), String::from("null")),
    ]
}

fn blank(_: &Ctx) -> Vec<Entry> {
    vec![plain(String::new(), String::from("null"))]
}

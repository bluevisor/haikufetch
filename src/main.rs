use std::{io::IsTerminal, process::ExitCode};

use haikufetch::{
    cache,
    cli::{self, ColorWhen},
    config,
    logo::{self, Tint},
    modules::{self, Ctx, MODULES},
    paths, platform, render,
    style::{self, Painter},
};

fn main() -> ExitCode {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(cli) => cli,
        Err(message) => {
            eprintln!("haikufetch: {message}");
            return ExitCode::from(2);
        },
    };

    if cli.help {
        print!("{}", cli::HELP);
        return ExitCode::SUCCESS;
    }
    if cli.version {
        println!("haikufetch {}", env!("CARGO_PKG_VERSION"));
        return ExitCode::SUCCESS;
    }
    if cli.list_modules {
        for m in MODULES {
            println!("{:<11} {}", m.name, m.about);
        }
        return ExitCode::SUCCESS;
    }
    if cli.list_logos {
        for l in logo::LOGOS {
            println!("{}", l.id);
        }
        let custom = logo::user_logos();
        if !custom.is_empty() {
            let dir = logo::user_logo_dir()
                .map(|d| d.display().to_string())
                .unwrap_or_default();
            println!("\ncustom, in {dir}:");
            for name in custom {
                println!("{name}");
            }
        }
        return ExitCode::SUCCESS;
    }
    if cli.print_config {
        print!("{}", config::TEMPLATE);
        return ExitCode::SUCCESS;
    }
    if cli.init_config {
        return init_config(cli.config.as_deref());
    }
    if cli.clear_cache {
        cache::clear();
    }

    let mut config = config::load(cli.config.as_deref());
    for warning in &config.warnings {
        eprintln!("haikufetch: {warning}");
    }
    if let Some(padding) = cli.padding {
        config.padding = padding;
    }

    let names: Vec<String> = match (&cli.modules, cli.all) {
        (Some(list), _) => list.clone(),
        (None, true) => MODULES
            .iter()
            .map(|m| m.name.to_string())
            .filter(|m| m != "break")
            .collect(),
        (None, false) => config.modules.clone(),
    };
    let selected: Vec<_> = names
        .iter()
        .filter_map(|name| {
            let module = modules::find(name);
            if module.is_none() {
                eprintln!("haikufetch: unknown module {name:?}, see --list-modules");
            }
            module
        })
        .collect();

    let painter = painter(cli.color, cli.json);
    let ctx = Ctx {
        config: &config,
        painter,
    };

    if cli.json {
        let outputs = modules::run(&selected, &ctx);
        print!("{}", render::json(&outputs));
        cache::flush();
        return ExitCode::SUCCESS;
    }

    let logo_setting = cli.logo.clone().unwrap_or_else(|| config.logo.clone());
    let tint = logo::parse_tint(&config.logo_color);
    let (logo, auto_key) = pick_logo(&logo_setting, tint, &config.logo_colors, &painter);
    let key_color = match config.key_color.trim().to_ascii_lowercase().as_str() {
        "none" | "off" | "false" => None,
        "auto" | "" => auto_key,
        other => style::parse_color(other).or(auto_key),
    };
    render::set_key_color(key_color);

    let outputs = modules::run(&selected, &ctx);
    let lines = render::lines(&outputs, &painter);
    if render::print(logo.as_ref(), &lines, config.padding).is_err() {
        // a closed pipe (haikufetch | head) isn't worth an error message
        return ExitCode::SUCCESS;
    }
    if cli.timings {
        for o in &outputs {
            eprintln!(
                "{:>9.3} ms  {}{}",
                o.took.as_secs_f64() * 1000.0,
                o.module.name,
                if o.module.slow { " (thread)" } else { "" }
            );
        }
    }
    cache::flush();
    ExitCode::SUCCESS
}

/// Color is on for a terminal unless NO_COLOR is set or TERM is dumb; --color overrides
fn painter(when: ColorWhen, json: bool) -> Painter {
    let var = |name: &str| std::env::var(name).ok();
    let enabled = !json
        && match when {
            ColorWhen::Always => true,
            ColorWhen::Never => false,
            ColorWhen::Auto => {
                std::io::stdout().is_terminal()
                    && var("NO_COLOR").is_none_or(|v| v.is_empty())
                    && var("TERM").as_deref() != Some("dumb")
            },
        };
    let truecolor =
        style::detect_truecolor(var("COLORTERM").as_deref(), var("TERM_PROGRAM").as_deref());
    Painter { enabled, truecolor }
}

/// The logo to draw and the color "auto" labels take next to it. A name is looked up in the
/// custom logos folder first (so a custom logo can replace a built-in one), then the built-ins
fn pick_logo(
    setting: &str,
    tint: Tint,
    custom: &[style::Color],
    painter: &Painter,
) -> (Option<logo::Rendered>, Option<style::Color>) {
    let solid = match tint {
        Tint::Solid(color) => Some(color),
        _ => None,
    };
    let palette: Vec<(style::Color, style::Color)> = custom.iter().map(|&c| (c, c)).collect();
    let custom_key = solid.or_else(|| custom.first().copied());

    let setting = setting.trim();
    if setting.eq_ignore_ascii_case("none") || setting.is_empty() {
        return (None, solid);
    }
    let file = if logo::is_path(setting) {
        let art = logo::load_file(setting);
        if art.is_none() {
            eprintln!("haikufetch: can't read logo file {setting:?}, drawing the system's own");
        }
        art
    } else {
        logo::user_logo(setting)
    };
    if let Some(art) = file {
        let white = [(style::Color::Ansi(15), style::Color::Ansi(15))];
        let colors = if palette.is_empty() {
            &white[..]
        } else {
            &palette[..]
        };
        return (
            Some(logo::render(&art, colors, false, tint, painter)),
            custom_key,
        );
    }

    let system = || {
        let os = platform::os();
        logo::for_system(&os.id, &os.like)
    };
    let builtin = if setting.eq_ignore_ascii_case("auto") || logo::is_path(setting) {
        system()
    } else {
        logo::find(setting).or_else(|| {
            eprintln!(
                "haikufetch: no logo named {setting:?}, drawing the system's own (see --list-logos)"
            );
            system()
        })
    };
    let Some(builtin) = builtin else {
        return (None, solid);
    };
    let colors = if palette.is_empty() {
        builtin.colors
    } else {
        &palette[..]
    };
    let rendered = logo::render(builtin.art, colors, builtin.id == "macos", tint, painter);
    let key = if palette.is_empty() {
        logo::auto_key_color(Some(builtin), tint, painter)
    } else {
        custom_key
    };
    (Some(rendered), key)
}

fn init_config(path: Option<&std::path::Path>) -> ExitCode {
    let Some(path) = path
        .map(std::path::Path::to_path_buf)
        .or_else(|| paths::config_dir().map(|d| d.join("haikufetch/config.toml")))
    else {
        eprintln!("haikufetch: no home folder to put the config in");
        return ExitCode::FAILURE;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if path.exists() {
        let backup = path.with_extension("toml.bak");
        if std::fs::rename(&path, &backup).is_ok() {
            println!("Moved the old config to {}", backup.display());
        }
    }
    match std::fs::write(&path, config::TEMPLATE) {
        Ok(()) => {
            println!("Wrote {}", path.display());
            ExitCode::SUCCESS
        },
        Err(error) => {
            eprintln!("haikufetch: can't write {}: {error}", path.display());
            ExitCode::FAILURE
        },
    }
}

//! The shell haikufetch runs in, and its version, found the cheapest way that works:
//!
//! 1. Walk up the process tree to the first known shell (wrappers like sudo, make or a script in
//!    between are skipped), falling back to $SHELL.
//! 2. Version, cheapest first: the shell's own environment variable, the package path
//!    (Homebrew's `Cellar/fish/4.9.3`, Nix's `<hash>-fish-3.7.1`), the version string compiled into
//!    the binary (read from its string section only, a few KB), the cache, and only then
//!    `<shell> --version` (2-6 ms, killed after a second), whose answer is cached until the
//!    executable changes.

use std::{
    io::Read,
    os::unix::fs::MetadataExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::{binfmt, cache, info::Shell, platform};

const KNOWN_SHELLS: &[&str] = &[
    "fish", "zsh", "bash", "sh", "dash", "ash", "ksh", "mksh", "oksh", "tcsh", "csh", "nu",
    "elvish", "xonsh", "pwsh", "ion", "oil.ovm", "osh", "ysh", "yash",
];

/// Ancestors looked at before giving up. The shell is nearly always the parent, but sudo,
/// nix-shell or a task runner can sit in between
const MAX_DEPTH: usize = 16;

const VERSION_TIMEOUT: Duration = Duration::from_secs(1);

pub fn detect() -> Option<Shell> {
    let mut pid = std::os::unix::process::parent_id();
    for _ in 0..MAX_DEPTH {
        // pid 0 is the kernel. pid 1 is still looked at: in a container it can be the shell
        if pid == 0 {
            break;
        }
        let (ppid, process_name) = platform::process_parent(pid)?;
        let mut name = normalize(&process_name);
        if is_known(&name) {
            // argv[0] says how it was started: macOS' /bin/sh is bash under the hood
            if let Some(argv0) = platform::process_argv0(pid).map(|a| normalize(&a))
                && is_known(&argv0)
            {
                name = argv0;
            }
            let path = platform::process_path(pid);
            let version = version(&name, path.as_deref());
            return Some(Shell {
                name: pretty(&name),
                version,
            });
        }
        pid = ppid;
    }

    // Not started from a shell (a launcher, cron, an IDE task): the login shell
    let login = std::env::var("SHELL").ok().filter(|s| !s.is_empty())?;
    let name = normalize(&login);
    if !is_known(&name) {
        return Some(Shell {
            name,
            version: None,
        });
    }
    // The resolved path carries Homebrew's version (/opt/homebrew/bin/fish is a symlink)
    let path = std::fs::canonicalize(&login)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or(login);
    let version = version(&name, Some(&path));
    Some(Shell {
        name: pretty(&name),
        version,
    })
}

/// The terminal emulator as a process: the first ancestor above the shell that isn't another
/// shell or a wrapper. For terminals that don't announce themselves in the environment
pub fn terminal_process() -> Option<String> {
    const WRAPPERS: &[&str] = &[
        "sudo",
        "su",
        "doas",
        "login",
        "script",
        "nix-shell",
        "env",
        "time",
        "haikufetch",
    ];
    let mut pid = std::os::unix::process::parent_id();
    let mut seen_shell = false;
    for _ in 0..MAX_DEPTH {
        if pid <= 1 {
            return None;
        }
        let (ppid, process_name) = platform::process_parent(pid)?;
        let name = normalize(&process_name);
        if is_known(&name) {
            seen_shell = true;
        } else if seen_shell && !WRAPPERS.contains(&name.as_str()) {
            return Some(pretty_terminal(&name));
        }
        pid = ppid;
    }
    None
}

fn pretty_terminal(name: &str) -> String {
    match name {
        "gnome-terminal-server" | "gnome-terminal-" | "gnome-terminal" => "GNOME Terminal",
        "kgx" => "GNOME Console",
        "ptyxis-agent" | "ptyxis" => "Ptyxis",
        "konsole" => "Konsole",
        "alacritty" => "Alacritty",
        "kitty" => "kitty",
        "foot" | "footclient" => "foot",
        "xfce4-terminal" => "Xfce Terminal",
        "tilix" => "Tilix",
        "terminator" => "Terminator",
        "wezterm-gui" => "WezTerm",
        "sshd" | "sshd-session" => "SSH",
        "tmux: server" | "tmux" => "tmux",
        "screen" | "SCREEN" => "screen",
        "Terminal" => "Terminal",
        other => other,
    }
    .to_string()
}

/// A process name or path as a bare shell name: no directories, no login '-', no Nix
/// ".name-wrapped"
pub fn normalize(name: &str) -> String {
    let base = name
        .rsplit('/')
        .next()
        .unwrap_or(name)
        .trim_start_matches('-');
    let base = base
        .strip_prefix('.')
        .and_then(|b| b.strip_suffix("-wrapped"))
        .unwrap_or(base);
    base.to_string()
}

pub fn is_known(name: &str) -> bool {
    KNOWN_SHELLS.contains(&name)
}

/// The names people use, as fastfetch shows them
pub fn pretty(name: &str) -> String {
    match name {
        "nu" => "nushell",
        "pwsh" => "PowerShell",
        "oil.ovm" => "oils",
        other => other,
    }
    .to_string()
}

pub fn version(name: &str, path: Option<&str>) -> Option<String> {
    // sh is whatever the system links (bash on macOS, dash on Debian): a version would mislead
    if name == "sh" {
        return None;
    }
    let env = match name {
        "nu" => Some("NU_VERSION"),
        "xonsh" => Some("XONSH_VERSION"),
        "pwsh" => Some("POWERSHELL_VERSION"),
        _ => None,
    };
    if let Some(v) = env
        .and_then(|var| std::env::var(var).ok())
        .and_then(|v| parse_version(&v))
    {
        return Some(v);
    }

    let path = path?;
    if let Some(v) = package_path_version(path) {
        return Some(v);
    }
    if let Some(marker) = binary_marker(name)
        && let Some(v) =
            binfmt::string_section(path).and_then(|data| version_after_marker(&data, marker))
    {
        return Some(v);
    }
    if matches!(name, "dash" | "ash" | "mksh" | "oksh") {
        return None;
    }

    // Valid while the executable keeps its size and modification time. The value "-" records a
    // shell that printed no version, so it isn't run again for nothing
    let stamp = std::fs::metadata(path)
        .ok()
        .map(|m| format!("{}:{}", m.len(), m.mtime_nsec() + m.mtime() * 1_000_000_000));
    let key = format!("shell:{name}:{path}");
    if let (Some(stamp), Some(cached)) = (&stamp, cache::get(&key))
        && let Some((cached_stamp, version)) = cached.split_once(' ')
        && cached_stamp == stamp
    {
        return (version != "-").then(|| version.to_string());
    }

    // xonsh's executable is python
    let program = if name == "xonsh" { "xonsh" } else { path };
    let output = run_version(program)?;
    let version = if name == "ksh" {
        ksh_version(&output).or_else(|| parse_version(&output))
    } else {
        parse_version(&output)
    };
    if let Some(stamp) = stamp {
        cache::set(
            &key,
            &format!("{stamp} {}", version.as_deref().unwrap_or("-")),
        );
    }
    version
}

/// Runs `<program> --version`, returning stdout then stderr (ksh prints on stderr). A shell
/// slower than VERSION_TIMEOUT is killed and counts as printing nothing; None if it can't start
fn run_version(program: &str) -> Option<String> {
    let mut child = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + VERSION_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_micros(250)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Some(String::new());
            },
        }
    }
    let mut output = Vec::new();
    if let Some(mut out) = child.stdout.take() {
        let _ = out.read_to_end(&mut output);
    }
    output.push(b'\n');
    if let Some(mut err) = child.stderr.take() {
        let _ = err.read_to_end(&mut output);
    }
    Some(String::from_utf8_lossy(&output).into_owned())
}

/// ksh93's release isn't a dotted number: "(AT&T Research) 93u+ 2012-08-01" gives "93u+"
pub fn ksh_version(output: &str) -> Option<String> {
    let (_, rest) = output.split_once("(AT&T Research) ")?;
    rest.split_whitespace().next().map(str::to_string)
}

/// The first dotted number in a version banner: "5.3.20" in "GNU bash, version 5.3.20(1)-release
/// (aarch64-apple-darwin25.6.0)". Numbers glued to a word ("darwin25.6.0") are skipped, a
/// leading 'v' is allowed
pub fn parse_version(output: &str) -> Option<String> {
    output
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '.')
        .find_map(|token| {
            let token = token.strip_prefix('v').unwrap_or(token);
            let end = token
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(token.len());
            let version = token[..end].trim_end_matches('.');
            (version.contains('.') && version.split('.').all(|p| !p.is_empty()))
                .then(|| version.to_string())
        })
}

/// Homebrew's ".../Cellar/fish/4.9.3_1/bin/fish" gives "4.9.3", Nix's
/// "/nix/store/<hash>-fish-3.7.1/bin/fish" gives "3.7.1". The whole path part must be the version
pub fn package_path_version(path: &str) -> Option<String> {
    let exact = |candidate: &str| {
        (parse_version(candidate).as_deref() == Some(candidate)).then(|| candidate.to_string())
    };
    if let Some((_, rest)) = path.split_once("/Cellar/") {
        let keg = rest.split('/').nth(1)?;
        return exact(keg.split('_').next()?);
    }
    let (_, rest) = path.split_once("/nix/store/")?;
    let (_, name_version) = rest.split('/').next()?.split_once('-')?;
    // the version starts after the first '-' followed by a digit, as Nix's parseDrvName splits
    let version = name_version.match_indices('-').find_map(|(i, _)| {
        name_version
            .get(i + 1..)
            .filter(|r| r.starts_with(|c: char| c.is_ascii_digit()))
    })?;
    exact(version)
}

/// What precedes the version in a shell's string literals, the same markers fastfetch uses
fn binary_marker(name: &str) -> Option<&'static [u8]> {
    match name {
        "bash" => Some(b"@(#)Bash version "),
        "zsh" => Some(b"zsh-"),
        "ash" => Some(b"BusyBox "),
        _ => None,
    }
}

/// Finds a C string starting with `marker` and parses the version after it. Matches not at the
/// start of a string, or not followed by a version ("zsh-newuser-install"), are skipped
pub fn version_after_marker(data: &[u8], marker: &[u8]) -> Option<String> {
    let mut from = 0;
    while let Some(found) = find_bytes(data.get(from..)?, marker) {
        let start = from + found;
        from = start + 1;
        if start > 0 && data[start - 1] != 0 {
            continue;
        }
        let rest = data.get(start + marker.len()..)?;
        let literal: Vec<u8> = rest
            .iter()
            .take(64)
            .take_while(|&&b| b != 0)
            .copied()
            .collect();
        if let Some(version) = parse_version(&String::from_utf8_lossy(&literal)) {
            return Some(version);
        }
    }
    None
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    let (&first, rest) = needle.split_first()?;
    let mut offset = 0;
    while let Some(found) = haystack.get(offset..)?.iter().position(|&b| b == first) {
        let start = offset + found;
        if haystack.get(start + 1..)?.starts_with(rest) {
            return Some(start);
        }
        offset = start + 1;
    }
    None
}

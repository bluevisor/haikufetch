//! Command-line flags. Hand-rolled: a dozen flags don't need an argument-parsing crate, and
//! skipping one keeps startup and the binary small.

use std::path::PathBuf;

pub const HELP: &str = "\
haikufetch - fast system information

Usage: haikufetch [options]

Output:
  -a, --all                 Show every module
  -m, --modules <list>      Show these modules in this order, e.g. os,cpu,memory
      --json                Print the information as JSON instead
      --color <when>        auto (default), always or never. NO_COLOR is honored
  -l, --logo <name|path>    Draw this logo: a name from --list-logos, a file, or none
      --no-logo             Same as --logo none
  -p, --padding <n>         Spaces between the logo and the information (default 3)

Config:
  -c, --config <path>       Read this config file instead of the default one
      --init-config         Write a commented default config file and exit
      --print-config        Print the default config to stdout and exit
      --clear-cache         Forget cached shell versions, Wi-Fi names and GPU names

Info:
      --list-modules        List every module with a short description
      --list-logos          List the built-in logos
      --timings             Print how long each module took, to stderr
  -h, --help                Print this help
  -V, --version             Print the version
";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorWhen {
    #[default]
    Auto,
    Always,
    Never,
}

#[derive(Debug, Default)]
pub struct Cli {
    pub all: bool,
    pub modules: Option<Vec<String>>,
    pub json: bool,
    pub color: ColorWhen,
    pub logo: Option<String>,
    pub padding: Option<usize>,
    pub config: Option<PathBuf>,
    pub init_config: bool,
    pub print_config: bool,
    pub clear_cache: bool,
    pub list_modules: bool,
    pub list_logos: bool,
    pub timings: bool,
    pub help: bool,
    pub version: bool,
}

/// Parses the arguments after the program name. Accepts "--flag value" and "--flag=value"
pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<Cli, String> {
    let mut cli = Cli::default();
    let mut args = args.into_iter();

    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_string(), Some(value.to_string()))
            },
            _ => (arg.clone(), None),
        };
        let mut value = |name: &str| -> Result<String, String> {
            inline
                .clone()
                .or_else(|| args.next())
                .ok_or_else(|| format!("{name} needs a value"))
        };

        match flag.as_str() {
            "-a" | "--all" => cli.all = true,
            "-m" | "--modules" => {
                let list = value(&flag)?;
                cli.modules = Some(
                    list.split([',', ' '])
                        .map(str::trim)
                        .filter(|m| !m.is_empty())
                        .map(str::to_string)
                        .collect(),
                );
            },
            "--json" => cli.json = true,
            "--color" | "--colour" => {
                cli.color = match value(&flag)?.as_str() {
                    "auto" => ColorWhen::Auto,
                    "always" | "yes" | "on" => ColorWhen::Always,
                    "never" | "no" | "off" => ColorWhen::Never,
                    other => {
                        return Err(format!(
                            "--color takes auto, always or never, not {other:?}"
                        ));
                    },
                }
            },
            "-l" | "--logo" => cli.logo = Some(value(&flag)?),
            "--no-logo" => cli.logo = Some(String::from("none")),
            "-p" | "--padding" => {
                let padding = value(&flag)?;
                cli.padding = Some(
                    padding
                        .parse()
                        .map_err(|_| format!("--padding takes a number, not {padding:?}"))?,
                );
            },
            "-c" | "--config" | "--config-file" => cli.config = Some(PathBuf::from(value(&flag)?)),
            "--init-config" | "--reset-config" => cli.init_config = true,
            "--print-config" => cli.print_config = true,
            "--clear-cache" => cli.clear_cache = true,
            "--list-modules" => cli.list_modules = true,
            "--list-logos" => cli.list_logos = true,
            "--timings" => cli.timings = true,
            "-h" | "--help" => cli.help = true,
            "-V" | "--version" => cli.version = true,
            other => return Err(format!("unknown option {other:?}, see haikufetch --help")),
        }
    }
    Ok(cli)
}

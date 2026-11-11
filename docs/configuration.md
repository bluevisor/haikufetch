# Configuration

haikufetch reads one TOML file. Every setting is optional, and a setting that is misspelled or has
the wrong type is reported once on stderr and skipped, while the rest of the file still applies.

| System | Path |
|---|---|
| macOS, Linux | `$XDG_CONFIG_HOME/haikufetch/config.toml`, default `~/.config/haikufetch/config.toml` |
| Haiku | `~/config/settings/haikufetch/config.toml` |

`--config <path>` reads another file. `--init-config` writes the commented defaults (moving an
existing file to `config.toml.bak`), and `--print-config` prints them.

If there is no haikufetch config, a rustfetch 0.x config is read from where 0.x kept it
(`~/.config/rustfetch/`, or `~/Library/Application Support/rustfetch/` on macOS). Its `[display]`
switches become the module list it describes.

## `modules`

The lines to show, top to bottom. Default:

```toml
modules = [
    "title", "separator",
    "os", "host", "kernel", "uptime", "packages", "shell", "terminal",
    "screen", "cpu", "gpu", "memory", "swap", "disk",
    "local_ip", "ipv6", "wifi", "tailscale", "battery",
    "break", "colors",
]
```

| Module | Shows | Hidden when |
|---|---|---|
| `title` | `user@host` | never |
| `separator` | a dashed line as wide as the title | never |
| `os` | name, version and architecture | never |
| `host` | the machine's model | the firmware doesn't say |
| `kernel` | kernel name and release (the hrev on Haiku) | never |
| `uptime` | time since boot, `2d 3h 14m` | never |
| `packages` | count per package manager: brew, MacPorts, dpkg, rpm, pacman, apk, xbps, flatpak, snap, pkgman | no manager found |
| `shell` | the shell haikufetch was started from, and its version | never |
| `terminal` | the terminal emulator | it can't be told |
| `screen` | each display's resolution and refresh rate | no display (SSH to a headless box) |
| `cpu` | name, core counts (`6+12` on hybrid chips) and top frequency | never |
| `gpu` | each GPU, with core count and top clock where known | none found |
| `memory` | RAM in use, as Activity Monitor or `free` count it | never |
| `swap` | swap in use, or `Disabled` | never |
| `disk` | space used on the filesystem holding each `[disk]` path | the path doesn't exist |
| `local_ip` | IPv4 address and prefix of the interface carrying the default route | offline |
| `ipv6` | a global IPv6 address of that interface, stable ones before privacy ones | IPv4-only networks |
| `wifi` | the Wi-Fi network's name | not on Wi-Fi |
| `tailscale` | Tailscale's IPv4 address and machine name | Tailscale isn't connected |
| `battery` | charge, state and (macOS) the power adapter | no battery |
| `power` | power drawn from the battery (Linux) | not reported |
| `locale` | `LC_ALL`, `LC_MESSAGES` or `LANG` | none set |
| `colors` | the terminal's 16 colors | color is off |
| `break` | an empty line | never |

`-a` shows every module, `-m a,b,c` a list of your own. Old names still work: `ram`, `ipv4`,
`identifier`, `power_draw`, `tailscale_ip`, `display`.

## `[logo]`

| Key | Default | |
|---|---|---|
| `name` | `"auto"` | your system's logo. A name from `--list-logos`, `"none"`, or a path to a text file (`"~/art.txt"`) |
| `colors` | none | colors for the art's `$1`, `$2`, ... markers, replacing the logo's own |
| `color` | `"default"` | the logo's own colors. `"rainbow"` draws the six-color Apple logo. A color name (`"bright blue"`) or hex code (`"#ff8800"`) paints the whole logo |
| `padding` | `3` | spaces between the logo and the text |

An unknown Linux distribution draws the logo of the first `ID_LIKE` entry haikufetch knows, then
Tux.

### Custom logos

Save art as `~/.config/haikufetch/logos/<name>.txt` (`~/config/settings/haikufetch/logos/` on
Haiku) and set `name = "<name>"`; `--list-logos` lists them, and one named like a built-in
replaces it. In the art, `$1` to `$9` switch to the 1st to 9th of `colors` (text before the first
marker takes color 1); without `colors` it is drawn white. With `key_color = "auto"`, labels take
color 1. [examples/logos](../examples/logos) has a worked example, the Seen Health ginkgo:

```toml
[logo]
name = "seen"
colors = ["#a7e2c0", "#5dc6b2", "#00a7a2", "#759e86", "#418b7d", "#007571"]
```

## `[style]`

| Key | Default | |
|---|---|---|
| `key_color` | `"auto"` | label color. `"auto"` follows the logo (the Apple logo's yellow stripe on macOS, the leaf green on Haiku), `"none"` keeps plain bold labels |
| `bars` | `true` | draw usage bars after memory, swap and disk figures; off shows `(64%)` |
| `bar_width` | `10` | cells in a usage bar (1-50) |

Colors are names (`black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`, `gray`,
each with `bright` in front) or hex codes. On terminals without 24-bit color, hex codes become the
nearest of the 256 standard colors.

## Module settings

```toml
[cpu]
frequency = true      # the "@ 4.61 GHz" part

[gpu]
frequency = true

[screen]
refresh_rate = true

[disk]
paths = ["/"]         # one line per path; "/" means /boot on Haiku

[tailscale]
name = true           # one DNS query to Tailscale's resolver, well under 1 ms

[battery]
adapter = true        # name the power adapter while plugged in (macOS)
```

## `[keys]`

Renames a module's label: `memory = "Memory"`, `local_ip = "IP"`. For `disk`, `{path}` stands
for the path: `disk = "Disk {path}"`.

## Caches

Answers that are slow to get and rarely change are kept in `~/.cache/haikufetch/cache.tsv`
(`~/config/cache/haikufetch/` on Haiku): shell versions (until the shell's binary changes), GPU
names from pci.ids, and package counts read from big databases (until the database changes).
`--clear-cache` forgets them.

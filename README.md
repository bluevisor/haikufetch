# haikufetch

System information at a glance, for **macOS**, **Linux** and **Haiku**. A neofetch/fastfetch-style
tool that starts, gathers everything and exits in about the time it takes your terminal to draw it.

```
                    ▄█▌          jane@studio
                 ▄████           -----------
                █████▀           OS: macOS Tahoe 26.7.1 (aarch64)
               ▐██▀▀             Host: MacBook Pro (16-inch, M5 Pro)
       ▄▄▄▄        ▄▄▄▄▄▄        Kernel: Darwin 25.6.0
   ▄▄█████████▄▄▄██████████▄     Uptime: 2d 3h 14m
  ▓█████████████████████████▀    Packages: 335 (brew), 9 (brew-cask)
 █████████████████████████▓      Shell: zsh 5.9
▓████████████████████████▒       Terminal: Ghostty 1.3.1
█████████████████████████        Screen: 3456x2234 @ 120Hz
█████████████████████████        CPU: Apple M5 Pro (6+12) @ 4.61 GHz
█████████████████████████▒       GPU: Apple M5 Pro (20 cores) @ 1.62 GHz
▒██████████████████████████▄     RAM: 41.4 GiB / 64 GiB ██████▍░░░ 64%
 ████████████████████████████▄   Swap: 1.2 GiB / 4 GiB ██▉░░░░░░░ 30%
  ███████████████████████████▓   Disk: 512 GiB / 926 GiB █████▌░░░░ 55%
   █████████████████████████▓    Local IPv4 (en0): 192.168.1.20/24
    ▓██████████████████████▀     Wi-Fi: Home
     ▀█████████▀▀████████▓       Tailscale: 100.81.13.5 (studio)
       ▀▀▀▀▀▀      ▀▀▀▀▀▀        Battery: 80% (Charging, 140W USB-C Power Adapter)
```

## Why another one

- **Fast.** ~4.6 ms on an M5 MacBook Pro with every module on, against ~37 ms for fastfetch and
  ~19 ms for rustfetch 0.3, the project this grew out of. Slow probes (a program, the network, a
  framework) run on their own threads, and nothing slow runs twice: shell versions, GPU names and
  package counts are cached until the thing they describe changes.
- **Small.** Two dependencies (`libc`, `toml`), a ~700 KB static binary. No X11, Wayland, Vulkan,
  wgpu or AppKit: displays come from IOKit on macOS, DRM and EDID on Linux, app_server on Haiku.
- **Things other fetches skip.** The Wi-Fi network's name (macOS without Location Services, Linux
  without NetworkManager, Haiku), Tailscale's address and machine name, the power adapter, hybrid
  core counts (6+12), Apple GPU core counts and clocks, Jetson and Raspberry Pi GPUs.
- **Scriptable.** `--json` prints the same facts as data.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/bluevisor/haikufetch/main/install.sh | bash
```

or from source (Rust 1.88+):

```bash
cargo install --git https://github.com/bluevisor/haikufetch
```

On Haiku, build natively: `pkgman install rust_bin && cargo install --git https://github.com/bluevisor/haikufetch`.

## Use

| | |
|---|---|
| `haikufetch` | the default modules |
| `haikufetch -a` | every module |
| `haikufetch -m os,cpu,memory` | just these, in this order |
| `haikufetch --json` | the facts as JSON |
| `haikufetch -l arch` / `--no-logo` | another logo, or none |
| `haikufetch --list-modules` | what each module shows |
| `haikufetch --timings` | how long each module took |
| `haikufetch --init-config` | write a commented config file |

`haikufetch --help` lists everything. Color follows the terminal: off when piped, off with
`NO_COLOR`, forced with `--color always`.

## Configure

The config lives at `~/.config/haikufetch/config.toml` (`~/config/settings/haikufetch/` on Haiku).
There is none until you ask for one with `haikufetch --init-config`; every setting is optional.

```toml
modules = ["title", "separator", "os", "cpu", "gpu", "memory", "disk", "battery"]

[logo]
name = "auto"        # the system's logo; a name from --list-logos, "none", or your own art
color = "default"    # "rainbow" for the six-color Apple, or any color: "red", "#ff8800"

[style]
key_color = "auto"   # labels follow the logo; "none" or any color
bars = true

[disk]
paths = ["/", "/home"]

[keys]
memory = "Memory"    # rename any label
```

Your own logo goes in `~/.config/haikufetch/logos/<name>.txt` with `name = "<name>"`;
[examples/logos](examples/logos) shows one with its own colors.
[docs/configuration.md](docs/configuration.md) has every setting. Config files from rustfetch 0.x
still work as they are.

## How it works

[docs/architecture.md](docs/architecture.md) explains where each fact comes from on each system
and why, including the shortcuts that keep it fast.

## License

MIT. Grew out of [lemuray/rustfetch](https://github.com/lemuray/rustfetch) and was rewritten
from scratch.

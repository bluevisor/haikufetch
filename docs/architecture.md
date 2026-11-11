# How haikufetch works

## Shape

```
main.rs        flags, config, logo, color; runs the modules and prints
modules.rs     one function per line: platform facts -> label, colored value, JSON
platform/      one backend per OS, all with the same functions
  macos.rs     sysctl, libproc, IOKit, CoreFoundation; CoreGraphics, configd, CoreWLAN on demand
  linux.rs     /proc and /sys only
  haiku.rs     the kernel API, plus haiku_shim.cpp for app_server and the network kit
unix.rs        what every Unix answers the same way (user, host, uname, statfs, terminal)
net.rs         getifaddrs and the rules that pick the address to show
tailscale.rs   the Tailscale tunnel and a hand-built MagicDNS reverse lookup
shell.rs       the process-tree walk and the cheapest way to each shell's version
binfmt.rs      the string section of Mach-O and ELF binaries
logo.rs        built-in art (ascii/*.txt), $1..$9 color markers, the rainbow Apple
config.rs      the TOML file, read key by key, and the rustfetch 0.x translation
cache.rs       one key/value file for slow answers, replaced atomically
style.rs, format.rs, render.rs   colors, number formatting, layout and JSON
```

Platform code returns plain data (`info.rs`); only `modules.rs` formats it, so the text and the
JSON never disagree.

## Speed

A run is a process start, a handful of system calls and one write. What keeps it near 4 ms:

- **Threads for anything that waits.** Modules flagged `slow` (packages, shell, screen, gpu, wifi,
  tailscale, battery) run on scoped threads; the main thread does the cheap ones meanwhile. The
  slowest one sets the total.
- **Load frameworks late.** Only CoreFoundation and IOKit are linked on macOS. CoreGraphics,
  SystemConfiguration, Foundation and CoreWLAN are `dlopen`ed by the module that needs them, on
  its thread.
- **Read less.** A shell's version comes from its binary's string section (~25 KB of zsh's
  1.3 MB), not from running it. Disk space is one `statfs`, not a volume list.
- **Cache what's slow and stable.** Running `fish --version`, scanning pci.ids, counting rpm or
  dpkg packages: each is done once and cached against the file's size and modification time.

Measure with `haikufetch --timings`, or `hyperfine -N haikufetch`.

## Where each fact comes from

### macOS

| Fact | Source | Notes |
|---|---|---|
| OS | `kern.osproductversion`, `kern.osversion` | codename from a table (Big Sur also reports itself as 10.16) |
| Host | `IODeviceTree:/product` `product-name` | Apple Silicon; Intel falls back to `hw.model` |
| CPU | `machdep.cpu.brand_string`, `hw.perflevelN.physicalcpu` | top clock from the power manager's `voltage-states5-sram` (P-cores) |
| GPU | `IOAccelerator` services: `model`, `gpu-core-count` | clock from `voltage-states9-sram`; Intel/AMD names from the PCI parent |
| Screens | `IOMobileFramebuffer` `DisplayWidth`/`Height`, `IOMFBDisplayRefresh` | ~0.2 ms. CoreGraphics' first call connects to the window server (~10 ms), so it is only the fallback for Intel Macs |
| Memory | `host_statistics64`: internal − purgeable + wired + compressed | the same "Memory Used" as Activity Monitor |
| Swap | `vm.swapusage` | |
| Battery | `AppleSmartBattery`, key by key | the whole property table is ~7x slower; `AdapterDetails` can outlive an unplug, so it is read only while `ExternalConnected` |
| Shell | `proc_pidinfo(PROC_PIDT_SHORTBSDINFO)` up the tree | the short info works on other users' processes, so the walk passes sudo and login; argv[0] from `KERN_PROCARGS2` tells `/bin/sh` from the bash it runs |
| Default route | `NET_RT_FLAGS` dump, the unscoped `RTF_GATEWAY` default | macOS keeps a scoped default per interface too |
| IPv6 state | `SIOCGIFAFLAG_IN6` | tentative, duplicated, detached and deprecated addresses are never shown |
| Wi-Fi | `SIOCGIFMEDIA` finds the joined interface; configd's `CachedScanRecord` names it | ~1 ms, and works without Location Services. CoreWLAN (~18 ms) is the fallback, and without Location Services it can only guess the last joined network |

### Linux

| Fact | Source | Notes |
|---|---|---|
| OS | `/etc/os-release` (or `/usr/lib/os-release`) | `ID_LIKE` picks a logo for derivatives |
| Host | DMI `product_name` / `product_version`, else the device tree `model` | firmware placeholders ("To be filled by O.E.M.") are skipped |
| CPU | `/proc/cpuinfo`, `cpufreq/cpuinfo_max_freq` | ARM cores named from implementer and part; big.LITTLE shows as `4+4` |
| GPU | PCI display-class devices, names from pci.ids (cached) | SoC GPUs from devfreq or DRM device-tree names (Jetson, Raspberry Pi, Mali, Adreno) |
| Screens | `/sys/class/drm/*/status`, `modes`, `edid` | the EDID's preferred timing gives refresh and monitor name; no X11 or Wayland needed |
| Memory | `/proc/meminfo`: total − `MemAvailable` | what `free` calls used |
| Battery | `/sys/class/power_supply`, type Battery, not scope Device | peripherals (mice, headsets) are skipped |
| Shell | `/proc/<pid>/stat`, `exe`, `cmdline` | a `(deleted)` exe after an upgrade still opens through `/proc/<pid>/exe` |
| Default route | `/proc/net/route`, `/proc/net/ipv6_route`, lowest metric | |
| Wi-Fi | `SIOCGIWESSID` on interfaces with a `wireless` directory | cfg80211 still answers the wireless-extensions call for every modern driver |
| Packages | dpkg status, pacman's local db, apk, xbps, flatpak, snap, Linuxbrew; rpm via `rpm -qa` | all cached against the database's modification time |

### Haiku

| Fact | Source | Notes |
|---|---|---|
| OS, kernel | `uname`: release (`R1~beta6` shown as `R1/beta6`) and the hrev | |
| CPU | `get_cpu_topology_info`, cpuid brand string on x86_64 | arm64 cores named from the MIDR; Apple's hypervisor hides the part, which leaves "Apple CPU" |
| GPU, screen | `BScreen` through a `BApplication` (`haiku_shim.cpp`) | asked once for both lines |
| Wi-Fi | `BNetworkDevice::GetNextAssociatedNetwork` | |
| Memory, swap | `get_system_info` | |
| Disk | `/boot` for `/` | `/` is a small virtual root |
| Shell | `_kern_process_info(pid, PARENT_ID)`, team args, the app image | the same private call libroot's `getppid` makes |
| Packages | `.hpkg` files in `/boot/system/packages` and `~/config/packages` | |

### Everywhere

- **Interfaces**: one `getifaddrs` per run, up and running, not loopback.
- **Local IPv4**: the address on the default route's interface. When that interface only has
  Tailscale's address (an exit node), the first physical interface instead.
- **Tailscale**: a 100.64/10 address on `tailscale0`, or on a `utun` that also carries
  Tailscale's `fd7a:115c:a1e0::/48` (WARP and Meshnet use 100.64/10 too). The name is one PTR
  query to 100.100.100.100, built and parsed by hand, labels limited to letters, digits, `-` and
  `_` so a reply can't inject escape codes.
- **Shell version**, cheapest first: the shell's own variable (`NU_VERSION`, ...), the install
  path (Homebrew Cellar, Nix store), the binary's string literals (`zsh-`, `@(#)Bash version `,
  `BusyBox `), the cache, then `<shell> --version` with a one-second limit.

## Adding a module

1. Add the fact to `info.rs` if it is new, and a function to each `platform/*.rs` (return `None`
   where a system can't answer).
2. Add a function and a `Module` entry in `modules.rs`; mark it `slow` if it runs a program,
   touches the network or loads a framework.
3. Add it to `DEFAULT_MODULES` and the template in `config.rs` if it should show by default, and
   to the table in `docs/configuration.md`.
4. Put parsing in a pure function and test it in `tests/`.

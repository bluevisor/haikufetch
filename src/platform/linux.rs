//! Linux: everything comes from /proc and /sys, no programs are run and no libraries loaded. The
//! GPU name is looked up in pci.ids once and cached; displays are read from DRM connectors and
//! their EDID, so they work without X11 or Wayland libraries (and show nothing over SSH on a
//! headless box, which is right).

use std::{ffi::c_char, fs, net::Ipv6Addr, path::Path, sync::OnceLock};

use crate::{cache, info::*, net::V6State};

fn read(path: impl AsRef<Path>) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let text = text
        .trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .to_string();
    (!text.is_empty()).then_some(text)
}

fn read_u64(path: impl AsRef<Path>) -> Option<u64> {
    read(path)?.parse().ok()
}

fn hex_id(path: impl AsRef<Path>) -> Option<u16> {
    u16::from_str_radix(read(path)?.trim_start_matches("0x"), 16).ok()
}

// ---- OS, host, uptime ----------------------------------------------------------------------

/// Parses os-release's KEY=value lines, unquoting values
pub fn parse_os_release(text: &str) -> Os {
    let mut values = std::collections::HashMap::new();
    for line in text.lines() {
        if let Some((key, value)) = line.split_once('=') {
            let value = value.trim().trim_matches(|c| c == '"' || c == '\'');
            values.insert(key.trim().to_string(), value.to_string());
        }
    }
    let get = |key: &str| values.get(key).filter(|v| !v.is_empty()).cloned();
    let id = get("ID")
        .unwrap_or_else(|| String::from("linux"))
        .to_ascii_lowercase();
    let pretty = get("PRETTY_NAME")
        .or_else(|| {
            get("NAME").map(|n| {
                format!("{n} {}", get("VERSION").unwrap_or_default())
                    .trim()
                    .to_string()
            })
        })
        .unwrap_or_else(|| String::from("Linux"));
    Os {
        id,
        like: get("ID_LIKE")
            .map(|l| l.split_whitespace().map(str::to_ascii_lowercase).collect())
            .unwrap_or_default(),
        pretty,
        version: get("VERSION_ID"),
        codename: get("VERSION_CODENAME"),
        build: get("BUILD_ID"),
    }
}

pub fn os() -> Os {
    static OS: OnceLock<Os> = OnceLock::new();
    OS.get_or_init(|| {
        fs::read_to_string("/etc/os-release")
            .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
            .map(|text| parse_os_release(&text))
            .unwrap_or_else(|_| Os {
                id: "linux".into(),
                pretty: "Linux".into(),
                ..Os::default()
            })
    })
    .clone()
}

/// Firmware placeholders vendors leave in DMI
fn is_junk(value: &str) -> bool {
    const JUNK: &[&str] = &[
        "to be filled by o.e.m.",
        "default string",
        "system product name",
        "system version",
        "none",
        "not applicable",
        "not specified",
        "o.e.m.",
        "oem",
        "x.x",
        "type1productconfigid",
        "123456789",
        "unknown",
        "invalid",
    ];
    let v = value.trim().to_ascii_lowercase();
    v.is_empty() || JUNK.contains(&v.as_str())
}

/// The DMI product (PCs, laptops, VMs), or the device tree model (Raspberry Pi, Jetson)
pub fn host_model() -> Option<String> {
    let dmi =
        |name: &str| read(format!("/sys/devices/virtual/dmi/id/{name}")).filter(|v| !is_junk(v));
    if let Some(product) = dmi("product_name") {
        // ThinkPads keep the readable name in product_version
        if let Some(version) =
            dmi("product_version").filter(|v| v.contains(' ') || v.starts_with("ThinkPad"))
        {
            return Some(version);
        }
        let vendor = dmi("sys_vendor").filter(|v| !product.starts_with(v.as_str()));
        return Some(match vendor {
            Some(vendor) if product.len() < 12 => format!("{vendor} {product}"),
            _ => product,
        });
    }
    read("/proc/device-tree/model").or_else(|| read("/sys/firmware/devicetree/base/model"))
}

pub fn uptime_seconds() -> Option<u64> {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: fills the timespec
    (unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut time) } == 0)
        .then_some(time.tv_sec as u64)
}

// ---- CPU -----------------------------------------------------------------------------------

struct CoreInfo {
    model: Option<String>,
    part: Option<(u32, u32)>,
    package: Option<u32>,
    core: Option<u32>,
}

fn parse_cpuinfo(text: &str) -> (Vec<CoreInfo>, Option<String>) {
    let mut cores = Vec::new();
    let mut hardware = None;
    let hex = |v: &str| u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok();
    for block in text.split("\n\n") {
        let mut info = CoreInfo {
            model: None,
            part: None,
            package: None,
            core: None,
        };
        let mut implementer = None;
        let mut is_processor = false;
        for line in block.lines() {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "processor" => is_processor = true,
                "model name" | "cpu model" => info.model = Some(value.to_string()),
                "CPU implementer" => implementer = hex(value),
                "CPU part" => info.part = implementer.zip(hex(value)),
                "physical id" => info.package = value.parse().ok(),
                "core id" => info.core = value.parse().ok(),
                "Hardware" | "Model" => hardware = Some(value.to_string()),
                _ => {},
            }
        }
        if is_processor {
            cores.push(info);
        }
    }
    (cores, hardware)
}

pub fn cpu() -> Option<Cpu> {
    let (cores, hardware) = parse_cpuinfo(&fs::read_to_string("/proc/cpuinfo").ok()?);
    let model = cores
        .iter()
        .find_map(|c| c.model.clone())
        .filter(|m| !m.starts_with("ARMv") && !m.is_empty());

    // ARM: cores grouped by type, biggest first ("4+4" on an RK3588)
    let mut parts: Vec<((u32, u32), u32)> = Vec::new();
    for part in cores.iter().filter_map(|c| c.part) {
        match parts.iter_mut().find(|(p, _)| *p == part) {
            Some((_, count)) => *count += 1,
            None => parts.push((part, 1)),
        }
    }
    parts.sort_by_key(|((_, part), _)| std::cmp::Reverse(*part));

    let vendor = parts
        .first()
        .and_then(|((implementer, _), _)| crate::arm::vendor(*implementer))
        .map(|v| format!("{v} CPU"));
    let hardware = hardware.or(vendor);
    let name = match (model, parts.as_slice()) {
        (Some(model), _) => crate::format::clean_cpu_name(&model),
        (None, [((i, p), _), ..]) if parts.len() == 1 => crate::arm::core_name(*i, *p)
            .map(str::to_string)
            .unwrap_or_else(|| hardware.clone().unwrap_or_else(|| String::from("ARM"))),
        (None, many) if !many.is_empty() => {
            let names: Vec<&str> = many
                .iter()
                .filter_map(|((i, p), _)| crate::arm::core_name(*i, *p))
                .collect();
            if names.len() == many.len() {
                names.join(" + ")
            } else {
                hardware.clone().unwrap_or_else(|| String::from("ARM"))
            }
        },
        _ => hardware.unwrap_or_else(|| String::from("Unknown CPU")),
    };

    let counts = if parts.len() > 1 {
        parts.iter().map(|(_, count)| *count).collect()
    } else {
        // physical cores: distinct (package, core) pairs; without them, every processor
        let mut physical: Vec<(u32, u32)> = cores
            .iter()
            .filter_map(|c| Some((c.package.unwrap_or(0), c.core?)))
            .collect();
        physical.sort_unstable();
        physical.dedup();
        vec![if physical.is_empty() {
            cores.len() as u32
        } else {
            physical.len() as u32
        }]
    };

    let max_khz = fs::read_dir("/sys/devices/system/cpu")
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("cpu"))
        .filter_map(|e| read_u64(e.path().join("cpufreq/cpuinfo_max_freq")))
        .max()
        .unwrap_or(0);
    Some(Cpu {
        name,
        cores: counts,
        max_mhz: max_khz / 1000,
    })
}

// ---- GPU -----------------------------------------------------------------------------------

fn vendor_name(vendor: u16) -> Option<&'static str> {
    Some(match vendor {
        0x10de => "NVIDIA",
        0x1002 | 0x1022 => "AMD",
        0x8086 => "Intel",
        0x1af4 => "Virtio",
        0x15ad => "VMware",
        0x1234 => "QEMU",
        0x80ee => "VirtualBox",
        0x1a03 => "ASPEED",
        0x5143 => "Qualcomm",
        0x106b => "Apple",
        _ => return None,
    })
}

/// A device's name in pci.ids. The marketing name sits in brackets ("GA104 [GeForce RTX 3070]"),
/// so that part wins
pub fn pci_ids_name(ids: &str, vendor: u16, device: u16) -> Option<String> {
    let vendor_hex = format!("{vendor:04x}");
    let device_hex = format!("{device:04x}");
    let mut in_vendor = false;
    for line in ids.lines() {
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        if !line.starts_with('\t') {
            if in_vendor {
                // the next vendor: the device isn't listed
                return None;
            }
            in_vendor = line
                .get(..4)
                .is_some_and(|v| v.eq_ignore_ascii_case(&vendor_hex));
            continue;
        }
        if in_vendor && !line.starts_with("\t\t") {
            let line = &line[1..];
            if line
                .get(..4)
                .is_some_and(|d| d.eq_ignore_ascii_case(&device_hex))
            {
                let name = line.get(4..)?.trim();
                let name = match (name.find('['), name.rfind(']')) {
                    (Some(open), Some(close)) if open < close => &name[open + 1..close],
                    _ => name,
                };
                return Some(name.to_string());
            }
        }
    }
    None
}

fn pci_gpu_name(vendor: u16, device: u16) -> String {
    let key = format!("gpu:{vendor:04x}:{device:04x}");
    if let Some(name) = cache::get(&key) {
        return name;
    }
    let ids = [
        "/usr/share/hwdata/pci.ids",
        "/usr/share/misc/pci.ids",
        "/usr/share/pci.ids",
        "/run/current-system/sw/share/hwdata/pci.ids",
    ]
    .iter()
    .find_map(|p| fs::read_to_string(p).ok());
    let vendor_label = vendor_name(vendor);
    let name = match ids
        .as_deref()
        .and_then(|ids| pci_ids_name(ids, vendor, device))
    {
        Some(model) => match vendor_label {
            Some(v) if !model.starts_with(v) => format!("{v} {model}"),
            _ => model,
        },
        None => format!("{} GPU {device:04x}", vendor_label.unwrap_or("PCI")),
    };
    cache::set(&key, &name);
    name
}

/// SoC GPUs that aren't on PCI, by device tree compatible string
fn soc_gpu_name(compatible: &str) -> Option<String> {
    let first = compatible.split('\0').find(|s| !s.is_empty())?;
    Some(match first {
        "nvidia,ga10b" => "NVIDIA Orin (Ampere)".into(),
        "nvidia,gv11b" => "NVIDIA Xavier (Volta)".into(),
        "nvidia,gp10b" => "NVIDIA Tegra X2 (Pascal)".into(),
        "nvidia,gm20b" => "NVIDIA Tegra X1 (Maxwell)".into(),
        c if c.starts_with("brcm,bcm2711") => "Broadcom VideoCore VI".into(),
        c if c.starts_with("brcm,bcm2712") => "Broadcom VideoCore VII".into(),
        c if c.starts_with("brcm,bcm2835") => "Broadcom VideoCore IV".into(),
        c if c.starts_with("arm,mali") => format!("ARM Mali {}", c.trim_start_matches("arm,mali-")),
        c if c.starts_with("qcom,adreno") => {
            format!("Qualcomm Adreno {}", c.trim_start_matches("qcom,adreno-"))
        },
        c if c.starts_with("apple,agx") => "Apple GPU".into(),
        _ => return None,
    })
}

pub fn gpus() -> Vec<Gpu> {
    let mut gpus = Vec::new();
    if let Ok(devices) = fs::read_dir("/sys/bus/pci/devices") {
        let mut paths: Vec<_> = devices.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            // class 0x03xxxx: display controllers
            if !read(path.join("class")).is_some_and(|c| c.starts_with("0x03")) {
                continue;
            }
            let (Some(vendor), Some(device)) =
                (hex_id(path.join("vendor")), hex_id(path.join("device")))
            else {
                continue;
            };
            gpus.push(Gpu {
                name: pci_gpu_name(vendor, device),
                cores: None,
                max_mhz: 0,
            });
        }
    }
    if gpus.is_empty() {
        // devfreq names the SoC GPU and its top clock (Jetson's 17000000.gpu)
        if let Ok(entries) = fs::read_dir("/sys/class/devfreq") {
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = fs::read(path.join("device/of_node/compatible"))
                    .ok()
                    .and_then(|c| soc_gpu_name(&String::from_utf8_lossy(&c)))
                else {
                    continue;
                };
                let max_mhz = read_u64(path.join("max_freq")).unwrap_or(0) / 1_000_000;
                gpus.push(Gpu {
                    name,
                    cores: None,
                    max_mhz,
                });
            }
        }
    }
    if gpus.is_empty() {
        // Raspberry Pi and other DRM-only GPUs
        for card in 0..4 {
            if let Some(name) = fs::read(format!(
                "/sys/class/drm/card{card}/device/of_node/compatible"
            ))
            .ok()
            .and_then(|c| soc_gpu_name(&String::from_utf8_lossy(&c)))
                && !gpus.iter().any(|g: &Gpu| g.name == name)
            {
                gpus.push(Gpu {
                    name,
                    cores: None,
                    max_mhz: 0,
                });
            }
        }
    }
    gpus
}

// ---- displays ------------------------------------------------------------------------------

/// The preferred mode and monitor name from an EDID block: (width, height, refresh, name)
pub fn parse_edid(edid: &[u8]) -> Option<(u32, u32, Option<u32>, Option<String>)> {
    if edid.len() < 128 || edid[..8] != [0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0] {
        return None;
    }
    let mut name = None;
    for at in [72, 90, 108] {
        let d = &edid[at..at + 18];
        if d[..3] == [0, 0, 0] && d[3] == 0xfc {
            let text: String = d[5..]
                .iter()
                .take_while(|&&b| b != 0x0a)
                .map(|&b| b as char)
                .collect();
            let text = text.trim().to_string();
            if !text.is_empty() && text.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                name = Some(text);
            }
        }
    }
    let d = &edid[54..72];
    let clock = u16::from_le_bytes([d[0], d[1]]) as u64 * 10_000;
    if clock == 0 {
        return None;
    }
    let h_active = d[2] as u64 | ((d[4] as u64 & 0xf0) << 4);
    let h_blank = d[3] as u64 | ((d[4] as u64 & 0x0f) << 8);
    let v_active = d[5] as u64 | ((d[7] as u64 & 0xf0) << 4);
    let v_blank = d[6] as u64 | ((d[7] as u64 & 0x0f) << 8);
    let total = (h_active + h_blank) * (v_active + v_blank);
    let refresh = (total > 0).then(|| ((clock as f64) / total as f64).round() as u32);
    Some((h_active as u32, v_active as u32, refresh, name))
}

/// Connected DRM connectors. Size is the first (preferred) mode, refresh and name come from the
/// EDID when its preferred timing matches that size
pub fn screens() -> Vec<Screen> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut connectors: Vec<_> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().contains('-'))
        })
        .collect();
    connectors.sort();
    connectors
        .iter()
        .filter(|p| read(p.join("status")).as_deref() == Some("connected"))
        .filter_map(|p| {
            let edid = fs::read(p.join("edid")).ok().and_then(|e| parse_edid(&e));
            let mode = read(p.join("modes")).and_then(|m| {
                let first = m.lines().next()?.to_string();
                let (w, h) = first.split_once('x')?;
                Some((
                    w.parse::<u32>().ok()?,
                    h.trim_end_matches(|c: char| !c.is_ascii_digit())
                        .parse::<u32>()
                        .ok()?,
                ))
            });
            let (width, height) = mode.or(edid.as_ref().map(|e| (e.0, e.1)))?;
            let refresh_hz = edid
                .as_ref()
                .filter(|e| (e.0, e.1) == (width, height))
                .and_then(|e| e.2);
            let connector = p
                .file_name()?
                .to_string_lossy()
                .split_once('-')
                .map(|(_, c)| c.to_string())
                .unwrap_or_default();
            let name = edid.and_then(|e| e.3).unwrap_or(connector.clone());
            let primary = connector.starts_with("eDP")
                || connector.starts_with("LVDS")
                || connector.starts_with("DSI");
            Some(Screen {
                name,
                width,
                height,
                refresh_hz,
                primary,
            })
        })
        .collect()
}

// ---- memory, disk --------------------------------------------------------------------------

fn meminfo() -> Option<std::collections::HashMap<String, u64>> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    Some(
        text.lines()
            .filter_map(|line| {
                let (key, rest) = line.split_once(':')?;
                Some((
                    key.to_string(),
                    rest.split_whitespace().next()?.parse::<u64>().ok()? * 1024,
                ))
            })
            .collect(),
    )
}

/// Used is what isn't available: MemAvailable counts reclaimable cache, like `free` and fastfetch
pub fn memory() -> Option<Usage> {
    let m = meminfo()?;
    let total = *m.get("MemTotal")?;
    let available = m.get("MemAvailable").copied().unwrap_or_else(|| {
        m.get("MemFree").copied().unwrap_or(0) + m.get("Cached").copied().unwrap_or(0)
    });
    Some(Usage {
        used: total.saturating_sub(available),
        total,
    })
}

pub fn swap() -> Option<Usage> {
    let m = meminfo()?;
    let total = *m.get("SwapTotal")?;
    Some(Usage {
        used: total.saturating_sub(*m.get("SwapFree")?),
        total,
    })
}

pub fn disk_path(path: &str) -> String {
    path.to_string()
}

// ---- battery, power ------------------------------------------------------------------------

/// The system battery: type Battery, not a peripheral's (mice and headsets report scope Device)
fn battery_dir() -> Option<std::path::PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir("/sys/class/power_supply")
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort();
    dirs.into_iter().find(|d| {
        read(d.join("type")).as_deref() == Some("Battery")
            && read(d.join("scope")).as_deref() != Some("Device")
    })
}

pub fn battery(_with_adapter: bool) -> Option<Battery> {
    let dir = battery_dir()?;
    if read(dir.join("present")).as_deref() == Some("0") {
        return None;
    }
    let percent = read_u64(dir.join("capacity")).or_else(|| {
        let now = read_u64(dir.join("energy_now")).or_else(|| read_u64(dir.join("charge_now")))?;
        let full =
            read_u64(dir.join("energy_full")).or_else(|| read_u64(dir.join("charge_full")))?;
        (full > 0).then(|| now * 100 / full)
    })?;
    let charge = match read(dir.join("status")).as_deref() {
        Some("Charging") => Charge::Charging,
        Some("Discharging") => Charge::Discharging,
        Some("Full") => Charge::Full,
        Some("Not charging") => Charge::NotCharging,
        _ => Charge::Unknown,
    };
    Some(Battery {
        percent: percent.min(100) as u8,
        charge,
        adapter: None,
    })
}

/// power_now (µW), or current_now (µA) × voltage_now (µV) on batteries that report those
pub fn power_draw_watts() -> Option<f64> {
    let dir = battery_dir()?;
    if let Some(power) = read_u64(dir.join("power_now")) {
        return Some(power as f64 / 1e6);
    }
    let current = read_u64(dir.join("current_now"))? as f64;
    let voltage = read_u64(dir.join("voltage_now"))? as f64;
    Some(current * voltage / 1e12)
}

// ---- packages ------------------------------------------------------------------------------

fn count_entries(path: &str, skip: &[&str]) -> usize {
    fs::read_dir(path)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let name = e.file_name();
                    let name = name.to_string_lossy();
                    !name.starts_with('.') && !skip.contains(&name.as_ref())
                })
                .count()
        })
        .unwrap_or(0)
}

/// A count that is slow to make (a megabyte database, a program), cached until `stamp_path`
/// changes
fn cached_count(stamp_path: &str, compute: impl FnOnce() -> usize) -> usize {
    use std::os::unix::fs::MetadataExt;
    let Ok(meta) = fs::metadata(stamp_path) else {
        return 0;
    };
    let stamp = format!("{}.{}:{}", meta.mtime(), meta.mtime_nsec(), meta.len());
    let key = format!("packages:{stamp_path}");
    if let Some((cached_stamp, count)) = cache::get(&key).as_deref().and_then(|c| c.split_once(' '))
        && cached_stamp == stamp
        && let Ok(count) = count.parse()
    {
        return count;
    }
    let count = compute();
    cache::set(&key, &format!("{stamp} {count}"));
    count
}

/// Counts the lines starting with `prefix` (dpkg's status file is ~5 ms to scan)
fn count_lines(path: &str, prefix: &str) -> usize {
    cached_count(path, || {
        let Ok(data) = fs::read(path) else { return 0 };
        let needle = format!("\n{prefix}");
        data.starts_with(prefix.as_bytes()) as usize
            + data
                .windows(needle.len())
                .filter(|w| *w == needle.as_bytes())
                .count()
    })
}

/// rpm keeps its database in SQLite (or ndb on openSUSE), so the count comes from `rpm -qa` (~100 ms), run only when
/// the database changed
fn count_rpm() -> usize {
    let database = [
        "/var/lib/rpm/rpmdb.sqlite",
        "/usr/lib/sysimage/rpm/rpmdb.sqlite",
        "/usr/lib/sysimage/rpm/Packages.db",
        "/var/lib/rpm/Packages.db",
        "/var/lib/rpm/Packages",
    ]
    .into_iter()
    .find(|p| Path::new(p).exists());
    let Some(database) = database else { return 0 };
    cached_count(database, || {
        std::process::Command::new("rpm")
            .args(["-qa", "--queryformat", "x\\n"])
            .stderr(std::process::Stdio::null())
            .output()
            .map(|o| o.stdout.iter().filter(|&&b| b == b'\n').count())
            .unwrap_or(0)
    })
}

pub fn packages() -> Vec<Packages> {
    let home = crate::paths::home()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![
        Packages {
            manager: "dpkg",
            count: count_lines("/var/lib/dpkg/status", "Status: install ok installed"),
        },
        Packages {
            manager: "pacman",
            count: count_entries("/var/lib/pacman/local", &["ALPM_DB_VERSION"]),
        },
        Packages {
            manager: "rpm",
            count: count_rpm(),
        },
        Packages {
            manager: "apk",
            count: count_lines("/lib/apk/db/installed", "P:"),
        },
        Packages {
            manager: "xbps",
            count: count_lines("/var/db/xbps/pkgdb-0.38.plist", "\t<key>"),
        },
        Packages {
            manager: "flatpak",
            count: count_entries("/var/lib/flatpak/app", &[])
                + count_entries(&format!("{home}/.local/share/flatpak/app"), &[]),
        },
        Packages {
            manager: "snap",
            count: count_entries("/snap", &["bin", "README"]),
        },
        Packages {
            manager: "brew",
            count: count_entries("/home/linuxbrew/.linuxbrew/Cellar", &[]),
        },
    ]
}

// ---- processes -----------------------------------------------------------------------------

/// /proc/<pid>/stat's "pid (comm) state ppid ...". comm can hold spaces and parentheses, so it is
/// everything between the first '(' and the last ')'
pub fn parse_proc_stat(stat: &str) -> Option<(u32, String)> {
    let start = stat.find('(')? + 1;
    let end = stat.rfind(')')?;
    let comm = stat.get(start..end)?;
    let ppid = stat
        .get(end + 1..)?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()?;
    Some((ppid, comm.to_string()))
}

pub fn process_parent(pid: u32) -> Option<(u32, String)> {
    parse_proc_stat(&fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The executable. After a package upgrade the link reads "... (deleted)"; /proc/<pid>/exe
/// itself still opens the old binary
pub fn process_path(pid: u32) -> Option<String> {
    let link = format!("/proc/{pid}/exe");
    let target = fs::read_link(&link).ok()?.to_string_lossy().into_owned();
    Some(if target.ends_with(" (deleted)") {
        link
    } else {
        target
    })
}

pub fn process_argv0(pid: u32) -> Option<String> {
    let cmdline = fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let end = cmdline
        .iter()
        .position(|&b| b == 0)
        .unwrap_or(cmdline.len());
    (end > 0).then(|| String::from_utf8_lossy(&cmdline[..end]).into_owned())
}

// ---- routes --------------------------------------------------------------------------------

/// The lowest-metric default route's interface from /proc/net/route (metric in decimal)
pub fn parse_route_v4(text: &str) -> Option<String> {
    text.lines()
        .skip(1)
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let flags = u32::from_str_radix(f.get(3)?, 16).ok()?;
            let default = *f.get(1)? == "00000000" && *f.get(7)? == "00000000" && flags & 1 != 0;
            default
                .then(|| Some((f.get(6)?.parse::<u32>().ok()?, f[0].to_string())))
                .flatten()
        })
        .min()
        .map(|(_, interface)| interface)
}

/// The same from /proc/net/ipv6_route (metric and flags in hex), skipping the unreachable default
/// the kernel keeps on lo
pub fn parse_route_v6(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split_whitespace().collect();
            let flags = u32::from_str_radix(f.get(8)?, 16).ok()?;
            let interface = *f.get(9)?;
            let usable = flags & 1 != 0 && flags & 0x200 == 0 && interface != "lo";
            let default = f.first()?.bytes().all(|b| b == b'0') && *f.get(1)? == "00";
            (default && usable)
                .then(|| {
                    Some((
                        u32::from_str_radix(f.get(5)?, 16).ok()?,
                        interface.to_string(),
                    ))
                })
                .flatten()
        })
        .min()
        .map(|(_, interface)| interface)
}

pub fn default_route(v6: bool) -> Option<String> {
    static ROUTES: OnceLock<[Option<String>; 2]> = OnceLock::new();
    ROUTES.get_or_init(|| {
        [
            fs::read_to_string("/proc/net/route")
                .ok()
                .and_then(|t| parse_route_v4(&t)),
            fs::read_to_string("/proc/net/ipv6_route")
                .ok()
                .and_then(|t| parse_route_v6(&t)),
        ]
    })[v6 as usize]
        .clone()
}

/// The flags /proc/net/if_inet6 lists for an address
pub fn parse_if_inet6_flags(text: &str, interface: &str, address: &Ipv6Addr) -> Option<u32> {
    let wanted = u128::from(*address);
    text.lines().find_map(|line| {
        let f: Vec<&str> = line.split_whitespace().collect();
        let matches =
            u128::from_str_radix(f.first()?, 16).ok()? == wanted && *f.get(5)? == interface;
        matches
            .then(|| u32::from_str_radix(f.get(4)?, 16).ok())
            .flatten()
    })
}

pub fn ipv6_state(interface: &str, address: &Ipv6Addr) -> V6State {
    static TABLE: OnceLock<String> = OnceLock::new();
    let table = TABLE.get_or_init(|| fs::read_to_string("/proc/net/if_inet6").unwrap_or_default());
    match parse_if_inet6_flags(table, interface, address) {
        // deprecated, tentative, failed duplicate detection
        Some(flags) if flags & (0x20 | 0x40 | 0x08) != 0 => V6State::Unusable,
        Some(flags) if flags & 0x01 != 0 => V6State::Temporary,
        _ => V6State::Stable,
    }
}

// ---- Wi-Fi ---------------------------------------------------------------------------------

const SIOCGIWESSID: libc::c_ulong = 0x8b1b;

#[repr(C)]
struct IwPoint {
    pointer: *mut libc::c_void,
    length: u16,
    flags: u16,
}

#[repr(C)]
struct IwReq {
    name: [c_char; libc::IFNAMSIZ],
    essid: IwPoint,
    // the union is as big as a sockaddr + padding
    _pad: [u8; 16],
}

/// The network name through the wireless extensions ioctl, which cfg80211 still answers for every
/// modern driver: one call, no netlink, no NetworkManager
pub fn wifi_ssid() -> Option<String> {
    let mut interfaces: Vec<String> = fs::read_dir("/sys/class/net")
        .ok()?
        .flatten()
        .filter(|e| e.path().join("wireless").exists())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    // the one with the default route first
    let route = default_route(false);
    interfaces.sort_by_key(|i| Some(i) != route.as_ref());
    interfaces.iter().find_map(|i| essid(i))
}

fn essid(interface: &str) -> Option<String> {
    if interface.len() >= libc::IFNAMSIZ {
        return None;
    }
    let mut buffer = [0u8; 33];
    let mut request = IwReq {
        name: [0; libc::IFNAMSIZ],
        essid: IwPoint {
            pointer: buffer.as_mut_ptr() as *mut libc::c_void,
            length: buffer.len() as u16,
            flags: 0,
        },
        _pad: [0; 16],
    };
    for (slot, byte) in request.name.iter_mut().zip(interface.bytes()) {
        *slot = byte as c_char;
    }
    // SAFETY: the kernel writes at most `length` bytes into buffer
    let ok = unsafe {
        let socket = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if socket < 0 {
            return None;
        }
        let ok = libc::ioctl(socket, SIOCGIWESSID as _, &mut request) == 0;
        libc::close(socket);
        ok
    };
    if !ok {
        return None;
    }
    let length = (request.essid.length as usize).min(32);
    let name: String = String::from_utf8_lossy(&buffer[..length])
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    let name = name.trim().to_string();
    (!name.is_empty()).then_some(name)
}

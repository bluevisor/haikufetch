//! Haiku: the kernel's C API (system info, CPU topology, teams and images) for the basics, and a
//! small C++ shim (haiku_shim.cpp) for what only the kits know: the graphics card, the screen mode
//! and the Wi-Fi network.

use std::{
    ffi::{CStr, c_char, c_int},
    mem::MaybeUninit,
    net::Ipv6Addr,
    sync::OnceLock,
};

use crate::{info::*, net::V6State};

unsafe extern "C" {
    fn haikufetch_graphics(
        gpu: *mut c_char,
        gpu_size: usize,
        name: *mut c_char,
        name_size: usize,
        screen: *mut u32,
    ) -> c_int;
    fn haikufetch_wifi(ssid: *mut c_char, size: usize) -> c_int;
    /// Private, but how libroot's own getppid() works: 3 is PARENT_ID
    fn _kern_process_info(process: libc::pid_t, which: i32) -> libc::pid_t;
}

fn c_text(buffer: &[c_char]) -> Option<String> {
    // SAFETY: the shim and the kernel always NUL-terminate inside the buffer
    let text = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    (!text.is_empty()).then_some(text)
}

fn system_info() -> Option<libc::system_info> {
    let mut info = MaybeUninit::<libc::system_info>::zeroed();
    // SAFETY: fills the struct
    (unsafe { libc::get_system_info(info.as_mut_ptr()) } == 0)
        .then(|| unsafe { info.assume_init() })
}

// ---- OS, host, uptime ----------------------------------------------------------------------

/// "Haiku R1/beta6+development": uname's release, with the '~' Haiku uses in package versions
pub fn os() -> Os {
    let uname = crate::unix::uname();
    let release = uname
        .as_ref()
        .map(|u| u.release.replace('~', "/"))
        .filter(|r| !r.is_empty());
    let build = uname.and_then(|u| u.version.split_whitespace().next().map(str::to_string));
    let pretty = match &release {
        Some(release) => format!("Haiku {release}"),
        None => String::from("Haiku"),
    };
    Os {
        id: "haiku".into(),
        like: Vec::new(),
        pretty,
        version: release,
        codename: None,
        build,
    }
}

pub fn host_model() -> Option<String> {
    None
}

pub fn uptime_seconds() -> Option<u64> {
    // SAFETY: no arguments
    Some(unsafe { libc::system_time() } as u64 / 1_000_000)
}

// ---- CPU -----------------------------------------------------------------------------------

/// Vendor names in the order of Haiku's cpu_vendor enum. libc's copy of the enum stops early,
/// so the topology's raw number is looked up here
const VENDORS: [&str; 29] = [
    "",
    "AMD",
    "Cyrix",
    "IDT",
    "Intel",
    "National Semiconductor",
    "Rise",
    "Transmeta",
    "VIA",
    "IBM",
    "Motorola",
    "NEC",
    "Hygon",
    "Sun",
    "Fujitsu",
    "ARM",
    "Broadcom",
    "Cavium",
    "DEC",
    "HiSilicon",
    "Infineon",
    "Freescale",
    "NVIDIA",
    "Applied Micro",
    "Qualcomm",
    "Marvell",
    "Apple",
    "Ampere",
    "Microsoft",
];

const TOPOLOGY_CORE: u32 = 3;
const TOPOLOGY_PACKAGE: u32 = 4;
const TOPOLOGY_ROOT: u32 = 1;
const PLATFORM_ARM64: u32 = 7;

/// The topology tree as raw words, so an enum value libc doesn't know can't be undefined
/// behavior: (type, data words). A node is id, type, level, padding, then an 8-byte aligned union
/// (root: platform; package: vendor, cache line; core: model, then a u64 default frequency)
fn topology() -> Vec<(u32, [u32; 4])> {
    const _: () = assert!(std::mem::size_of::<libc::cpu_topology_node_info>() == 32);
    let mut count = 0u32;
    // SAFETY: a null buffer asks for the count
    if unsafe { libc::get_cpu_topology_info(std::ptr::null_mut(), &mut count) } != 0 || count == 0 {
        return Vec::new();
    }
    let mut nodes: Vec<libc::cpu_topology_node_info> = Vec::with_capacity(count as usize);
    // SAFETY: room for count nodes; only the first count written are used
    unsafe {
        if libc::get_cpu_topology_info(nodes.as_mut_ptr(), &mut count) != 0 {
            return Vec::new();
        }
        nodes.set_len(count as usize);
    }
    nodes
        .iter()
        .map(|node| {
            // SAFETY: plain-old-data, read as words
            let words: [u32; 8] =
                unsafe { std::ptr::read_unaligned(node as *const _ as *const [u32; 8]) };
            (words[1], [words[4], words[5], words[6], words[7]])
        })
        .collect()
}

pub fn cpu() -> Option<Cpu> {
    let nodes = topology();
    let cores = nodes
        .iter()
        .filter(|(kind, _)| *kind == TOPOLOGY_CORE)
        .count() as u32;
    let cores = if cores > 0 {
        cores
    } else {
        system_info().map(|i| i.cpu_count).unwrap_or(0)
    };
    let name = brand_string()
        .or_else(|| topology_name(&nodes))
        .unwrap_or_else(|| String::from("Unknown CPU"));

    // the running clock, else the core's nominal one (0 under QEMU and Apple's hypervisor)
    let mut info = MaybeUninit::<libc::cpu_info>::zeroed();
    // SAFETY: one cpu_info
    let current = if unsafe { libc::get_cpu_info(0, 1, info.as_mut_ptr()) } == 0 {
        unsafe { info.assume_init() }.current_frequency
    } else {
        0
    };
    let nominal = nodes
        .iter()
        .find(|(kind, _)| *kind == TOPOLOGY_CORE)
        .map(|(_, w)| (w[2] as u64) | ((w[3] as u64) << 32))
        .unwrap_or(0);
    let hz = if current > 0 { current } else { nominal };
    Some(Cpu {
        name,
        cores: vec![cores],
        max_mhz: hz / 1_000_000,
    })
}

#[cfg(target_arch = "x86_64")]
fn brand_string() -> Option<String> {
    use std::arch::x86_64::__cpuid;
    // SAFETY: cpuid is always available on x86_64
    let max = unsafe { __cpuid(0x8000_0000) }.eax;
    if max < 0x8000_0004 {
        return None;
    }
    let mut bytes = Vec::with_capacity(48);
    for leaf in 0x8000_0002u32..=0x8000_0004 {
        let r = unsafe { __cpuid(leaf) };
        for word in [r.eax, r.ebx, r.ecx, r.edx] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
    }
    let text = String::from_utf8_lossy(&bytes)
        .trim_matches(|c: char| c == '\0' || c.is_whitespace())
        .to_string();
    (!text.is_empty()).then(|| crate::format::clean_cpu_name(&text))
}

#[cfg(not(target_arch = "x86_64"))]
fn brand_string() -> Option<String> {
    None
}

/// "ARM Cortex-A72" from the package's vendor and the core's MIDR. Apple's hypervisor hides the
/// part number, which leaves "Apple CPU"
fn topology_name(nodes: &[(u32, [u32; 4])]) -> Option<String> {
    let platform = nodes
        .iter()
        .find(|(kind, _)| *kind == TOPOLOGY_ROOT)
        .map(|(_, w)| w[0]);
    let vendor = nodes
        .iter()
        .find(|(kind, _)| *kind == TOPOLOGY_PACKAGE)
        .map(|(_, w)| w[0])?;
    let vendor = VENDORS
        .get(vendor as usize)
        .copied()
        .filter(|v| !v.is_empty())?;
    let midr = nodes
        .iter()
        .find(|(kind, _)| *kind == TOPOLOGY_CORE)
        .map(|(_, w)| w[0]);
    let part = midr
        .filter(|_| platform == Some(PLATFORM_ARM64))
        .and_then(|m| crate::arm::core_name(m >> 24, (m >> 4) & 0xfff));
    Some(match part {
        Some(part) if part.starts_with(vendor) => part.to_string(),
        Some(part) => format!("{vendor} {part}"),
        None => format!("{vendor} CPU"),
    })
}

// ---- graphics ------------------------------------------------------------------------------

struct Graphics {
    gpu: Option<String>,
    screen: Option<Screen>,
}

/// Connecting to app_server costs a few ms, so it is asked once for both the GPU and screen lines
fn graphics() -> &'static Graphics {
    static GRAPHICS: OnceLock<Graphics> = OnceLock::new();
    GRAPHICS.get_or_init(|| {
        let mut gpu = [0 as c_char; 256];
        let mut name = [0 as c_char; 256];
        let mut mode = [0u32; 3];
        // SAFETY: buffer sizes are passed along
        let found = unsafe {
            haikufetch_graphics(
                gpu.as_mut_ptr(),
                gpu.len(),
                name.as_mut_ptr(),
                name.len(),
                mode.as_mut_ptr(),
            )
        };
        Graphics {
            gpu: (found & 1 != 0).then(|| c_text(&gpu)).flatten(),
            screen: (found & 2 != 0 && mode[0] > 0).then(|| Screen {
                name: c_text(&name).unwrap_or_default(),
                width: mode[0],
                height: mode[1],
                refresh_hz: (mode[2] > 0).then(|| (mode[2] + 500) / 1000),
                primary: true,
            }),
        }
    })
}

pub fn gpus() -> Vec<Gpu> {
    graphics()
        .gpu
        .clone()
        .map(|name| Gpu {
            name,
            cores: None,
            max_mhz: 0,
        })
        .into_iter()
        .collect()
}

pub fn screens() -> Vec<Screen> {
    graphics().screen.clone().into_iter().collect()
}

// ---- memory, disk, power -------------------------------------------------------------------

pub fn memory() -> Option<Usage> {
    let i = system_info()?;
    let page = libc::B_PAGE_SIZE as u64;
    Some(Usage {
        used: i.used_pages * page,
        total: i.max_pages * page,
    })
}

pub fn swap() -> Option<Usage> {
    let i = system_info()?;
    let page = libc::B_PAGE_SIZE as u64;
    Some(Usage {
        used: i.max_swap_pages.saturating_sub(i.free_swap_pages) * page,
        total: i.max_swap_pages * page,
    })
}

/// "/" is a small virtual root on Haiku; the system volume is /boot
pub fn disk_path(path: &str) -> String {
    if path == "/" {
        String::from("/boot")
    } else {
        path.to_string()
    }
}

pub fn battery(_with_adapter: bool) -> Option<Battery> {
    None
}

pub fn power_draw_watts() -> Option<f64> {
    None
}

pub fn packages() -> Vec<Packages> {
    let count = |dir: String| {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".hpkg"))
                    .count()
            })
            .unwrap_or(0)
    };
    let home = crate::paths::home()
        .map(|h| h.to_string_lossy().into_owned())
        .unwrap_or_default();
    vec![Packages {
        manager: "pkgman",
        count: count("/boot/system/packages".into()) + count(format!("{home}/config/packages")),
    }]
}

// ---- processes -----------------------------------------------------------------------------

fn team(pid: u32) -> Option<libc::team_info> {
    let mut info = MaybeUninit::<libc::team_info>::zeroed();
    // SAFETY: fills the struct
    (unsafe {
        libc::_get_team_info(
            pid as i32,
            info.as_mut_ptr(),
            std::mem::size_of::<libc::team_info>(),
        )
    } == 0)
        .then(|| unsafe { info.assume_init() })
}

/// The application image's path: a team's first B_APP_IMAGE
pub fn process_path(pid: u32) -> Option<String> {
    let mut cookie = 0i32;
    loop {
        // image_info holds function pointers, which can't be zeroed safely as a type
        let mut image = MaybeUninit::<libc::image_info>::uninit();
        // SAFETY: the kernel fills the struct on success
        let status = unsafe {
            libc::_get_next_image_info(
                pid as i32,
                &mut cookie,
                image.as_mut_ptr(),
                std::mem::size_of::<libc::image_info>(),
            )
        };
        if status != 0 {
            return None;
        }
        // SAFETY: filled by the successful call; only plain fields are read
        let image = unsafe { image.assume_init_ref() };
        if image.image_type == 1 {
            return c_text(&image.name);
        }
    }
}

/// The first word of the team's arguments (the kernel keeps 64 bytes of them)
pub fn process_argv0(pid: u32) -> Option<String> {
    let info = team(pid)?;
    c_text(&info.args)?
        .split_whitespace()
        .next()
        .map(str::to_string)
}

pub fn process_parent(pid: u32) -> Option<(u32, String)> {
    let path = process_path(pid).or_else(|| process_argv0(pid))?;
    let name = path.rsplit('/').next().unwrap_or(&path).to_string();
    // SAFETY: a plain syscall
    let parent = unsafe { _kern_process_info(pid as i32, 3) };
    Some((parent.max(0) as u32, name))
}

// ---- network -------------------------------------------------------------------------------

/// Haiku has no privacy addresses, and machines rarely have more than one interface: the first
/// physical one is used
pub fn default_route(_v6: bool) -> Option<String> {
    None
}

pub fn ipv6_state(_interface: &str, _address: &Ipv6Addr) -> V6State {
    V6State::Stable
}

pub fn wifi_ssid() -> Option<String> {
    let mut ssid = [0 as c_char; 64];
    // SAFETY: the size is passed along
    (unsafe { haikufetch_wifi(ssid.as_mut_ptr(), ssid.len()) } == 1)
        .then(|| c_text(&ssid))
        .flatten()
}

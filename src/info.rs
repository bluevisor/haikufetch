//! The facts haikufetch shows, as plain data. Platform code fills these in; `modules` turns them
//! into lines or JSON. Nothing here is formatted yet.

use std::net::IpAddr;

#[derive(Debug, Clone, Default)]
pub struct Os {
    /// os-release ID ("arch", "ubuntu"), or "macos" / "haiku". Picks the logo
    pub id: String,
    /// Other ids this system is like (os-release ID_LIKE), the logo fallback
    pub like: Vec<String>,
    /// What to show: "macOS Tahoe 26.7.1", "Arch Linux", "Haiku R1/beta5 (hrev58000)"
    pub pretty: String,
    pub version: Option<String>,
    pub codename: Option<String>,
    pub build: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Cpu {
    pub name: String,
    /// One count per core type on hybrid CPUs ([6, 12] for 6 P-cores + 12 E-cores), otherwise
    /// the physical core count
    pub cores: Vec<u32>,
    pub max_mhz: u64,
}

#[derive(Debug, Clone, Default)]
pub struct Gpu {
    pub name: String,
    pub cores: Option<u32>,
    pub max_mhz: u64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Screen {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_hz: Option<u32>,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Charge {
    Charging,
    Discharging,
    Full,
    /// Plugged in but holding its charge (macOS' optimized charging, a charge limit)
    NotCharging,
    Unknown,
}

impl Charge {
    pub fn label(self) -> &'static str {
        match self {
            Charge::Charging => "Charging",
            Charge::Discharging => "Discharging",
            Charge::Full => "Full",
            Charge::NotCharging => "AC Connected",
            Charge::Unknown => "Unknown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Battery {
    pub percent: u8,
    pub charge: Charge,
    /// The power adapter's name while plugged in: "140W USB-C Power Adapter"
    pub adapter: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterfaceAddress {
    pub interface: String,
    pub address: IpAddr,
    pub prefix: u32,
}

#[derive(Debug, Clone)]
pub struct Shell {
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Packages {
    pub manager: &'static str,
    pub count: usize,
}

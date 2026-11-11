//! Network interfaces through getifaddrs(3), shared by every platform, and the rules that pick
//! which address the IPv4 and IPv6 lines show. The interface list is read once per run (it is the
//! expensive part, ~150 µs with musl) and shared by the IP, Wi-Fi and Tailscale modules.

use std::{
    ffi::CStr,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::OnceLock,
};

use crate::{info::InterfaceAddress, platform};

/// How usable an IPv6 address is, from the kernel's flags
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V6State {
    Stable,
    /// A privacy address that rotates
    Temporary,
    /// Tentative, duplicated or deprecated: never shown
    Unusable,
}

pub fn interfaces() -> &'static [InterfaceAddress] {
    static LIST: OnceLock<Vec<InterfaceAddress>> = OnceLock::new();
    LIST.get_or_init(read_interfaces)
}

#[cfg(target_os = "haiku")]
const IFF_LINK_UP: libc::c_uint = 0x1000;
#[cfg(not(target_os = "haiku"))]
const IFF_LINK_UP: libc::c_uint = libc::IFF_RUNNING as libc::c_uint;

fn read_interfaces() -> Vec<InterfaceAddress> {
    let mut list = Vec::new();
    let mut head: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs allocates a list we walk read-only and free once
    unsafe {
        if libc::getifaddrs(&mut head) != 0 {
            return list;
        }
        let mut node = head;
        while !node.is_null() {
            let entry = &*node;
            node = entry.ifa_next;
            let flags = entry.ifa_flags as libc::c_uint;
            let up = libc::IFF_UP as libc::c_uint | IFF_LINK_UP;
            if flags & up != up || flags & libc::IFF_LOOPBACK as libc::c_uint != 0 {
                continue;
            }
            if entry.ifa_addr.is_null() || entry.ifa_name.is_null() {
                continue;
            }
            let interface = CStr::from_ptr(entry.ifa_name)
                .to_string_lossy()
                .into_owned();
            let address = match (*entry.ifa_addr).sa_family as libc::c_int {
                libc::AF_INET => {
                    let sin = std::ptr::read_unaligned(entry.ifa_addr as *const libc::sockaddr_in);
                    IpAddr::V4(Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)))
                },
                libc::AF_INET6 => {
                    let sin6 =
                        std::ptr::read_unaligned(entry.ifa_addr as *const libc::sockaddr_in6);
                    IpAddr::V6(Ipv6Addr::from(sin6.sin6_addr.s6_addr))
                },
                _ => continue,
            };
            let prefix = netmask_prefix(entry.ifa_netmask, address.is_ipv6());
            list.push(InterfaceAddress {
                interface,
                address,
                prefix,
            });
        }
        libc::freeifaddrs(head);
    }
    list
}

/// Counts the set bits of a netmask. BSDs trim IPv4 masks (sa_len 7 for a /24), so only sa_len
/// bytes are read there and missing ones count as zero
unsafe fn netmask_prefix(mask: *const libc::sockaddr, v6: bool) -> u32 {
    if mask.is_null() {
        return 0;
    }
    let (offset, full) = if v6 { (8, 16) } else { (4, 4) };
    #[cfg(any(target_os = "macos", target_os = "haiku"))]
    // SAFETY: the kernel set sa_len; the mask is never read past it
    let available = (unsafe { (*mask).sa_len } as usize)
        .saturating_sub(offset)
        .min(full);
    #[cfg(not(any(target_os = "macos", target_os = "haiku")))]
    let available = full;
    let bytes = mask as *const u8;
    // SAFETY: within the sockaddr as bounded above
    (0..available)
        .map(|i| unsafe { *bytes.add(offset + i) }.count_ones())
        .sum()
}

/// Interfaces that are tunnels, bridges or containers rather than a network the machine is on
pub fn is_virtual(interface: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "utun",
        "awdl",
        "llw",
        "bridge",
        "anpi",
        "ap",
        "gif",
        "stf", // macOS
        "docker",
        "veth",
        "br-",
        "virbr",
        "tailscale",
        "tun",
        "tap",
        "wg",
        "zt",
        "lxc", // Linux
    ];
    PREFIXES.iter().any(|p| interface.starts_with(p))
}

/// 2000::/3, the only addresses reachable from the internet (no link-local, ULA or Tailscale)
pub fn is_global_v6(address: &Ipv6Addr) -> bool {
    address.segments()[0] & 0xe000 == 0x2000
}

pub fn is_tailscale_v4(address: &Ipv4Addr) -> bool {
    let o = address.octets();
    o[0] == 100 && o[1] & 0xc0 == 64
}

pub fn is_tailscale_v6(address: &Ipv6Addr) -> bool {
    address.segments()[..3] == [0xfd7a, 0x115c, 0xa1e0]
}

/// Tailscale's tunnel: tailscale0 on Linux, a utun on macOS that also carries Tailscale's IPv6
/// range. A 100.64/10 address alone isn't enough, Cloudflare WARP and Meshnet use it too
pub fn is_tailscale_tunnel(interface: &str, list: &[InterfaceAddress]) -> bool {
    interface.starts_with("tailscale")
        || (interface.starts_with("utun")
            && list.iter().any(|a| {
                a.interface == interface
                    && matches!(a.address, IpAddr::V6(v6) if is_tailscale_v6(&v6))
            }))
}

fn is_tailscale_entry(list: &[InterfaceAddress], entry: &InterfaceAddress) -> bool {
    matches!(entry.address, IpAddr::V4(v4) if is_tailscale_v4(&v4))
        && is_tailscale_tunnel(&entry.interface, list)
}

/// The address a Local IP line shows. v4-only networks never ask the routing table for IPv6
pub fn local_address(v6: bool) -> Option<InterfaceAddress> {
    let list = interfaces();
    if v6 {
        if !list
            .iter()
            .any(|a| matches!(a.address, IpAddr::V6(v6) if is_global_v6(&v6)))
        {
            return None;
        }
        let routes: Vec<String> = [
            platform::default_route(true),
            platform::default_route(false),
        ]
        .into_iter()
        .flatten()
        .collect();
        select_v6(list, &routes, platform::ipv6_state)
    } else {
        select_v4(list, platform::default_route(false).as_deref())
    }
}

/// The IPv4 address on the interface with the default route. When there's none, or it only has
/// Tailscale's address (an exit node), the first physical interface with one
pub fn select_v4(list: &[InterfaceAddress], route: Option<&str>) -> Option<InterfaceAddress> {
    let candidates: Vec<&InterfaceAddress> = list
        .iter()
        .filter(|a| a.address.is_ipv4() && !is_tailscale_entry(list, a))
        .collect();
    route
        .and_then(|r| candidates.iter().find(|a| a.interface == r))
        .or_else(|| candidates.iter().find(|a| !is_virtual(&a.interface)))
        .map(|a| (*a).clone())
}

/// A global IPv6 address: on the IPv6 route's interface, then the IPv4 route's, then any physical
/// one. Stable addresses beat temporary ones. An interface without a usable one is skipped: a
/// tunnel carrying the route often only has ULAs
/// `routes` are the interfaces of the IPv6 and IPv4 default routes, in that order
pub fn select_v6(
    list: &[InterfaceAddress],
    routes: &[String],
    state: fn(&str, &Ipv6Addr) -> V6State,
) -> Option<InterfaceAddress> {
    let global = |a: &&InterfaceAddress| matches!(a.address, IpAddr::V6(v6) if is_global_v6(&v6));
    list.iter().find(global)?;

    let mut order: Vec<String> = routes.to_vec();
    for a in list.iter().filter(global) {
        if !is_virtual(&a.interface) && !order.contains(&a.interface) {
            order.push(a.interface.clone());
        }
    }
    order.iter().find_map(|interface| {
        let mut temporary = None;
        for a in list
            .iter()
            .filter(global)
            .filter(|a| &a.interface == interface)
        {
            let IpAddr::V6(v6) = a.address else { continue };
            match state(interface, &v6) {
                V6State::Stable => return Some(a.clone()),
                V6State::Temporary if temporary.is_none() => temporary = Some(a.clone()),
                _ => {},
            }
        }
        temporary
    })
}

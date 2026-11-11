#![allow(clippy::unwrap_used, clippy::expect_used)]

//! The parsing behind the shell, network and Tailscale lines, with made-up inputs.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use haikufetch::{
    info::InterfaceAddress,
    net::{self, V6State},
    shell, tailscale,
};

#[test]
fn shell_names_normalize() {
    assert_eq!(shell::normalize("-zsh"), "zsh");
    assert_eq!(shell::normalize("/opt/homebrew/bin/fish"), "fish");
    assert_eq!(shell::normalize(".fish-wrapped"), "fish");
    assert!(shell::is_known("nu") && !shell::is_known("python3"));
    assert_eq!(shell::pretty("nu"), "nushell");
    assert_eq!(shell::pretty("pwsh"), "PowerShell");
}

#[test]
fn shell_versions_parse() {
    assert_eq!(
        shell::parse_version("GNU bash, version 5.3.20(1)-release (aarch64-apple-darwin25.6.0)")
            .as_deref(),
        Some("5.3.20")
    );
    assert_eq!(
        shell::parse_version("zsh 5.9 (arm64-apple-darwin25.0)").as_deref(),
        Some("5.9")
    );
    assert_eq!(
        shell::parse_version("fish, version 4.9.3").as_deref(),
        Some("4.9.3")
    );
    assert_eq!(shell::parse_version("v0.104.0").as_deref(), Some("0.104.0"));
    assert_eq!(shell::parse_version("built 2024 (1)"), None);
    assert_eq!(
        shell::ksh_version("  version         sh (AT&T Research) 93u+ 2012-08-01").as_deref(),
        Some("93u+")
    );
}

#[test]
fn package_paths_carry_versions() {
    assert_eq!(
        shell::package_path_version("/opt/homebrew/Cellar/fish/4.9.3/bin/fish").as_deref(),
        Some("4.9.3")
    );
    assert_eq!(
        shell::package_path_version("/opt/homebrew/Cellar/bash/5.3.20_1/bin/bash").as_deref(),
        Some("5.3.20")
    );
    assert_eq!(
        shell::package_path_version("/opt/homebrew/Cellar/fish/HEAD-1a2b3c/bin/fish"),
        None
    );
    assert_eq!(
        shell::package_path_version("/nix/store/abc123-fish-3.7.1/bin/fish").as_deref(),
        Some("3.7.1")
    );
    assert_eq!(
        shell::package_path_version("/nix/store/abc123-bash-interactive-5.2p37/bin/bash"),
        None
    );
    assert_eq!(shell::package_path_version("/bin/zsh"), None);
}

#[test]
fn versions_are_found_after_markers_at_string_starts() {
    let data = b"\0junk zsh-9.9\0zsh-newuser-install\0zsh-5.9-0-g73d3173\0";
    assert_eq!(
        shell::version_after_marker(data, b"zsh-").as_deref(),
        Some("5.9")
    );
    assert_eq!(
        shell::version_after_marker(
            b"\0@(#)Bash version 5.3.20(1) release GNU\0",
            b"@(#)Bash version "
        )
        .as_deref(),
        Some("5.3.20")
    );
    assert_eq!(shell::version_after_marker(b"nothing here", b"zsh-"), None);
}

fn address(interface: &str, address: &str, prefix: u32) -> InterfaceAddress {
    InterfaceAddress {
        interface: interface.into(),
        address: address.parse().unwrap(),
        prefix,
    }
}

#[test]
fn ipv4_follows_the_default_route() {
    let list = [
        address("en0", "192.168.1.20", 24),
        address("en5", "10.0.0.2", 24),
    ];
    assert_eq!(net::select_v4(&list, Some("en5")).unwrap().interface, "en5");
    assert_eq!(net::select_v4(&list, None).unwrap().interface, "en0");
}

#[test]
fn an_exit_node_route_still_shows_the_real_network() {
    let list = [
        address("en0", "192.168.1.20", 24),
        address("utun4", "100.81.138.51", 32),
        address("utun4", "fd7a:115c:a1e0::1", 128),
    ];
    assert_eq!(
        net::select_v4(&list, Some("utun4")).unwrap().interface,
        "en0"
    );
}

#[test]
fn a_vpn_carrying_the_route_is_shown_but_skipped_as_a_fallback() {
    // Cloudflare WARP uses 100.64/10 too, but isn't Tailscale
    let list = [
        address("en0", "192.168.1.20", 24),
        address("utun5", "100.96.0.7", 32),
    ];
    assert_eq!(
        net::select_v4(&list, Some("utun5")).unwrap().interface,
        "utun5"
    );
    let only_vpn = [address("utun5", "100.96.0.7", 32)];
    assert!(net::select_v4(&only_vpn, None).is_none());
}

#[test]
fn virtual_interfaces() {
    for name in [
        "utun4",
        "awdl0",
        "docker0",
        "veth12",
        "br-1a2b",
        "tailscale0",
        "wg0",
        "bridge100",
    ] {
        assert!(net::is_virtual(name), "{name}");
    }
    for name in [
        "en0", "eth0", "eno1", "enp3s0", "wlan0", "wlP1p1s0", "br0", "bond0",
    ] {
        assert!(!net::is_virtual(name), "{name}");
    }
}

#[test]
fn global_ipv6_only() {
    let global = |s: &str| net::is_global_v6(&s.parse::<Ipv6Addr>().unwrap());
    assert!(global("2001:db8::1"));
    for s in [
        "fe80::1",
        "fd7a:115c:a1e0::1",
        "::1",
        "::",
        "ff02::1",
        "::ffff:1.2.3.4",
    ] {
        assert!(!global(s), "{s}");
    }
}

fn temporary_for_ending_in_beef(_: &str, address: &Ipv6Addr) -> V6State {
    match address.segments()[7] {
        0xbeef => V6State::Temporary,
        0xdead => V6State::Unusable,
        _ => V6State::Stable,
    }
}

#[test]
fn ipv6_prefers_stable_addresses_and_skips_interfaces_without_one() {
    let list = [
        address("utun4", "fd00::1", 64),
        address("en0", "2001:db8::beef", 64),
        address("en0", "2001:db8::dead", 64),
        address("en0", "2001:db8::1", 64),
    ];
    let routes = [String::from("utun4"), String::from("en0")];
    let picked = net::select_v6(&list, &routes, temporary_for_ending_in_beef).unwrap();
    assert_eq!(picked.address, "2001:db8::1".parse::<IpAddr>().unwrap());

    let only_temporary = [address("en0", "2001:db8::beef", 64)];
    assert!(net::select_v6(&only_temporary, &[], temporary_for_ending_in_beef).is_some());
    let only_unusable = [address("en0", "2001:db8::dead", 64)];
    assert!(net::select_v6(&only_unusable, &[], temporary_for_ending_in_beef).is_none());
}

#[test]
fn tailscale_ranges() {
    assert!(net::is_tailscale_v4(&Ipv4Addr::new(100, 81, 138, 51)));
    assert!(!net::is_tailscale_v4(&Ipv4Addr::new(100, 128, 0, 1)));
    assert!(net::is_tailscale_v6(&"fd7a:115c:a1e0::1".parse().unwrap()));
    let list = [
        address("utun4", "100.81.138.51", 32),
        address("utun4", "fd7a:115c:a1e0::1", 128),
        address("utun5", "100.96.0.7", 32),
    ];
    assert!(net::is_tailscale_tunnel("utun4", &list));
    assert!(!net::is_tailscale_tunnel("utun5", &list));
    assert!(net::is_tailscale_tunnel("tailscale0", &[]));
}

#[test]
fn ptr_queries_are_built_right() {
    let query = tailscale::ptr_query(Ipv4Addr::new(100, 81, 138, 51), 0x1234);
    assert_eq!(&query[..12], &[0x12, 0x34, 1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    assert_eq!(
        &query[12..],
        b"\x0251\x03138\x0281\x03100\x07in-addr\x04arpa\x00\x00\x0c\x00\x01"
    );
}

/// A reply to `query` with one PTR answer pointing back at the question name
fn reply(query: &[u8], name: &[u8], rcode: u8) -> Vec<u8> {
    let mut m = query.to_vec();
    m[2] = 0x81;
    m[3] = 0x80 | rcode;
    m[7] = 1;
    m.extend_from_slice(&[0xc0, 12, 0, 12, 0, 1, 0, 0, 0, 60]);
    m.extend_from_slice(&(name.len() as u16).to_be_bytes());
    m.extend_from_slice(name);
    m
}

#[test]
fn ptr_answers_parse() {
    let query = tailscale::ptr_query(Ipv4Addr::new(100, 81, 138, 51), 7);
    let ok = reply(&query, b"\x06laptop\x07tailnet\x02ts\x03net\x00", 0);
    assert_eq!(
        tailscale::parse_ptr_response(&ok, 7).as_deref(),
        Some("laptop.tailnet.ts.net")
    );
    // another query's id, an error code, a truncated packet
    assert_eq!(tailscale::parse_ptr_response(&ok, 8), None);
    assert_eq!(
        tailscale::parse_ptr_response(&reply(&query, b"\x00", 3), 7),
        None
    );
    for cut in 0..ok.len() {
        assert_eq!(
            tailscale::parse_ptr_response(&ok[..cut], 7),
            None,
            "cut at {cut}"
        );
    }
    // an escape code in a label
    let evil = reply(&query, b"\x04\x1b[31\x00", 0);
    assert_eq!(tailscale::parse_ptr_response(&evil, 7), None);
}

#[test]
fn dns_names_are_bounded() {
    let mut long = Vec::new();
    for _ in 0..3 {
        long.push(63);
        long.extend_from_slice(&[b'a'; 63]);
    }
    long.push(0);
    assert_eq!(tailscale::read_name(&long, 0).unwrap().0.len(), 191);
    let mut too_long = Vec::new();
    for _ in 0..5 {
        too_long.push(63);
        too_long.extend_from_slice(&[b'a'; 63]);
    }
    too_long.push(0);
    assert!(tailscale::read_name(&too_long, 0).is_none());
    // a pointer to itself
    assert!(tailscale::read_name(&[0xc0, 0], 0).is_none());
}

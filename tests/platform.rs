#![allow(clippy::unwrap_used, clippy::expect_used)]

//! Each backend's parsers with made-up inputs, and smoke tests against the machine running them.

use haikufetch::{platform, unix};

#[test]
fn the_basics_are_always_there() {
    let os = platform::os();
    assert!(!os.id.is_empty() && !os.pretty.is_empty());
    assert!(unix::kernel().is_some());
    assert!(unix::hostname().is_some());
    assert!(platform::uptime_seconds().is_some_and(|s| s > 0));
    let cpu = platform::cpu().expect("cpu");
    assert!(!cpu.name.is_empty() && !cpu.cores.is_empty());
    let memory = platform::memory().expect("memory");
    assert!(memory.total > 0 && memory.used <= memory.total);
    let disk = unix::filesystem_usage(&platform::disk_path("/")).expect("disk");
    assert!(disk.used <= disk.total);
    assert!(unix::filesystem_usage("/no/such/directory").is_none());
}

#[test]
fn our_own_process_is_found() {
    let (parent, name) = platform::process_parent(std::process::id()).expect("self");
    assert_eq!(parent, std::os::unix::process::parent_id());
    assert!(!name.is_empty());
    assert!(platform::process_path(std::process::id()).is_some());
    assert!(platform::process_parent(999_999_999).is_none());
}

#[cfg(target_os = "macos")]
mod macos {
    use haikufetch::{binfmt, platform::*};

    #[test]
    fn codenames() {
        assert_eq!(codename("26.7.1"), Some("Tahoe"));
        assert_eq!(codename("15.0"), Some("Sequoia"));
        assert_eq!(codename("10.16"), Some("Big Sur"));
        assert_eq!(codename("10.15.7"), Some("Catalina"));
        assert_eq!(codename("10.12.6"), None);
        assert_eq!(codename(""), None);
    }

    #[test]
    fn battery_math() {
        assert_eq!(battery_percent(4012, 5103), Some(79));
        assert_eq!(battery_percent(5200, 5103), Some(100));
        assert_eq!(battery_percent(1, 0), None);
        assert_eq!(battery_percent(i64::MAX, 100), None);
        assert_eq!(
            adapter_label(140, Some("140W USB-C Power Adapter")).as_deref(),
            Some("140W USB-C Power Adapter")
        );
        assert_eq!(adapter_label(96, None).as_deref(), Some("96W"));
        assert_eq!(
            adapter_label(30, Some("\x1b[31mEvil")).as_deref(),
            Some("30W ([31mEvil)")
        );
        assert_eq!(adapter_label(0, Some("x")), None);
    }

    #[test]
    fn power_manager_states() {
        let hz = [1_620_000_000u32, 800, 600_000_000, 700]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(max_state_mhz(&hz), Some(1620));
        let khz = [1_398_000u32, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect::<Vec<_>>();
        assert_eq!(max_state_mhz(&khz), Some(1398));
        assert_eq!(max_state_mhz(&[1, 2, 3]), None);
    }

    #[test]
    fn procargs() {
        assert_eq!(
            parse_procargs_argv0(b"\x01\0\0\0/bin/zsh\0\0\0-zsh\0").as_deref(),
            Some("-zsh")
        );
        assert_eq!(parse_procargs_argv0(b"\x01\0\0\0/bin/zsh"), None);
        assert_eq!(parse_procargs_argv0(b"\x01\0\0\0/bin/zsh\0\0\0"), None);
    }

    #[test]
    fn panels_and_ssids() {
        assert!(is_liquid_retina_xdr(0xa050, Some(60)));
        assert!(!is_liquid_retina_xdr(0xa053, Some(120)));
        assert!(is_liquid_retina_xdr(0xffff, Some(120)));
        assert_eq!(sanitize_ssid(" Home\u{9b} ").as_deref(), Some("Home"));
        assert_eq!(sanitize_ssid("<redacted>"), None);
        assert_eq!(
            sanitize_ssid("Not <redacted>").as_deref(),
            Some("Not <redacted>")
        );
    }

    #[test]
    fn launchd_is_visible_to_everyone() {
        assert_eq!(process_parent(1), Some((0, String::from("launchd"))));
    }

    #[test]
    fn zsh_carries_its_version_in_cstring() {
        let section = binfmt::string_section("/bin/zsh").expect("__cstring");
        assert!(section.len() < std::fs::metadata("/bin/zsh").unwrap().len() as usize / 10);
        assert!(haikufetch::shell::version_after_marker(&section, b"zsh-").is_some());
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::net::Ipv6Addr;

    use haikufetch::{binfmt, platform::*};

    #[test]
    fn os_release() {
        let os = parse_os_release(
            "NAME=\"Pop!_OS\"\nID=pop\nID_LIKE=\"ubuntu debian\"\nPRETTY_NAME=\"Pop!_OS 24.04 LTS\"\nVERSION_ID=\"24.04\"\n",
        );
        assert_eq!(os.id, "pop");
        assert_eq!(os.like, ["ubuntu", "debian"]);
        assert_eq!(os.pretty, "Pop!_OS 24.04 LTS");
        assert_eq!(parse_os_release("NAME=Foo\nVERSION=2\n").pretty, "Foo 2");
    }

    #[test]
    fn routes() {
        let v4 = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
                  wlan0\t00000000\t0107A8C0\t0003\t0\t0\t600\t00000000\n\
                  eth0\t00000000\t0101A8C0\t0003\t0\t0\t100\t00000000\n\
                  wg0\t00000000\t00000000\t0001\t0\t0\t50\t00000000\n";
        assert_eq!(parse_route_v4(v4).as_deref(), Some("wg0"));
        let v6 = "00000000000000000000000000000000 00 00000000000000000000000000000000 00 00000000000000000000000000000000 ffffffff 00000001 00000000 00200200 lo\n\
                  00000000000000000000000000000000 00 00000000000000000000000000000000 00 fe800000000000000000000000000001 00000400 00000001 00000000 00450003 eth0\n";
        assert_eq!(parse_route_v6(v6).as_deref(), Some("eth0"));
    }

    #[test]
    fn inet6_flags() {
        let table = "20010db8000000000000000000000001 02 40 00 01 eth0\n20010db8000000000000000000000002 02 40 00 20 eth0\n";
        let a: Ipv6Addr = "2001:db8::1".parse().unwrap();
        assert_eq!(parse_if_inet6_flags(table, "eth0", &a), Some(0x01));
        assert_eq!(parse_if_inet6_flags(table, "wlan0", &a), None);
    }

    #[test]
    fn proc_stat() {
        assert_eq!(
            parse_proc_stat("42 (tmux: server) S 1 42 42"),
            Some((1, String::from("tmux: server")))
        );
        assert_eq!(
            parse_proc_stat("7 (a) b) R 3 7"),
            Some((3, String::from("a) b")))
        );
        assert_eq!(parse_proc_stat("garbage"), None);
    }

    #[test]
    fn pci_ids() {
        let ids = "# comment\n10de  NVIDIA Corporation\n\t2484  GA104 [GeForce RTX 3070]\n\t\t1043 87b8  ROG\n\t1234  Plain Name\n8086  Intel Corporation\n\t46a6  Alder Lake-P GT2 [Iris Xe Graphics]\nC 03  Display controller\n";
        assert_eq!(
            pci_ids_name(ids, 0x10de, 0x2484).as_deref(),
            Some("GeForce RTX 3070")
        );
        assert_eq!(
            pci_ids_name(ids, 0x10de, 0x1234).as_deref(),
            Some("Plain Name")
        );
        assert_eq!(
            pci_ids_name(ids, 0x8086, 0x46a6).as_deref(),
            Some("Iris Xe Graphics")
        );
        assert_eq!(pci_ids_name(ids, 0x10de, 0x9999), None);
    }

    #[test]
    fn edid() {
        let mut edid = vec![0u8; 128];
        edid[..8].copy_from_slice(&[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
        // 1920x1080 @ 60: 148.5 MHz, 2200x1125 total
        edid[54..62].copy_from_slice(&[0x02, 0x3a, 0x80, 0x18, 0x71, 0x38, 0x2d, 0x40]);
        edid[72..77].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
        edid[77..85].copy_from_slice(b"DELL U27");
        edid[85] = 0x0a;
        assert_eq!(
            parse_edid(&edid),
            Some((1920, 1080, Some(60), Some(String::from("DELL U27"))))
        );
        assert_eq!(parse_edid(&edid[..100]), None);
    }

    #[test]
    fn our_binary_has_rodata() {
        let exe = std::env::current_exe().unwrap();
        let section = binfmt::string_section(exe.to_str().unwrap()).expect(".rodata");
        // black_box keeps the literal in the binary instead of folded into the comparison
        let marker = std::hint::black_box("haikufetch-rodata-marker-1.0").as_bytes();
        assert!(section.windows(marker.len()).any(|w| w == marker));
    }
}

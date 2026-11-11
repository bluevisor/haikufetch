//! ARM core names from the MIDR register's implementer and part numbers, for systems that only
//! report "ARMv8 Processor" (Linux on many boards, Haiku on arm64).

/// ARM core names by (implementer, part) from the MIDR, for CPUs whose /proc/cpuinfo only says
/// "ARMv8 Processor"
pub fn core_name(implementer: u32, part: u32) -> Option<&'static str> {
    Some(match (implementer, part) {
        (0x41, 0xd03) => "Cortex-A53",
        (0x41, 0xd04) => "Cortex-A35",
        (0x41, 0xd05) => "Cortex-A55",
        (0x41, 0xd07) => "Cortex-A57",
        (0x41, 0xd08) => "Cortex-A72",
        (0x41, 0xd09) => "Cortex-A73",
        (0x41, 0xd0a) => "Cortex-A75",
        (0x41, 0xd0b) => "Cortex-A76",
        (0x41, 0xd0c) => "Neoverse-N1",
        (0x41, 0xd0d) => "Cortex-A77",
        (0x41, 0xd40) => "Neoverse-V1",
        (0x41, 0xd41) => "Cortex-A78",
        (0x41, 0xd42) => "Cortex-A78AE",
        (0x41, 0xd44) => "Cortex-X1",
        (0x41, 0xd46) => "Cortex-A510",
        (0x41, 0xd47) => "Cortex-A710",
        (0x41, 0xd48) => "Cortex-X2",
        (0x41, 0xd49) => "Neoverse-N2",
        (0x41, 0xd4b) => "Cortex-A78C",
        (0x41, 0xd4f) => "Neoverse-V2",
        (0x41, 0xd80) => "Cortex-A520",
        (0x41, 0xd81) => "Cortex-A720",
        (0x41, 0xd82) => "Cortex-X4",
        (0x41, 0xd87) => "Cortex-A725",
        (0x41, 0xd85) => "Cortex-X925",
        (0x61, 0x022 | 0x023) => "Apple M1",
        (0x61, 0x024 | 0x025) => "Apple M1 Pro",
        (0x61, 0x028 | 0x029) => "Apple M1 Max",
        (0x61, 0x032 | 0x033) => "Apple M2",
        (0x51, 0x001) => "Qualcomm Oryon",
        _ => return None,
    })
}

/// The company behind a MIDR implementer code
pub fn vendor(implementer: u32) -> Option<&'static str> {
    Some(match implementer {
        0x41 => "ARM",
        0x42 => "Broadcom",
        0x43 => "Cavium",
        0x46 => "Fujitsu",
        0x48 => "HiSilicon",
        0x4e => "NVIDIA",
        0x50 => "Applied Micro",
        0x51 => "Qualcomm",
        0x53 => "Samsung",
        0x56 => "Marvell",
        0x61 => "Apple",
        0x69 => "Intel",
        0x6d => "Microsoft",
        0xc0 => "Ampere",
        _ => return None,
    })
}

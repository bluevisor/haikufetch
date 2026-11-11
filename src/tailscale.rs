//! Tailscale's address comes from the interface list. Its machine name comes from one reverse DNS
//! query to MagicDNS (100.100.100.100), which answers in ~0.3 ms, where `tailscale status --json`
//! takes ~30 ms and the LocalAPI needs a token.

use std::{
    net::{IpAddr, Ipv4Addr, UdpSocket},
    time::{Duration, Instant},
};

use crate::net;

/// MagicDNS answers in well under a millisecond; this only bounds a lost packet
const TIMEOUT: Duration = Duration::from_millis(25);

pub fn address() -> Option<Ipv4Addr> {
    let list = net::interfaces();
    list.iter().find_map(|a| match a.address {
        IpAddr::V4(v4)
            if net::is_tailscale_v4(&v4) && net::is_tailscale_tunnel(&a.interface, list) =>
        {
            Some(v4)
        },
        _ => None,
    })
}

/// The first label of the machine's MagicDNS name: "laptop" for laptop.tailnet.ts.net
pub fn machine_name(address: Ipv4Addr) -> Option<String> {
    let id = (std::process::id()
        ^ std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .subsec_nanos()) as u16;
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    // connected, so only MagicDNS' datagrams arrive
    socket.connect("100.100.100.100:53").ok()?;
    socket.send(&ptr_query(address, id)).ok()?;

    let deadline = Instant::now() + TIMEOUT;
    let mut buffer = [0u8; 512];
    // a stray reply (an earlier run's late answer) is skipped rather than ending the wait
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())?;
        socket.set_read_timeout(Some(left)).ok()?;
        let length = socket.recv(&mut buffer).ok()?;
        if let Some(name) = parse_ptr_response(&buffer[..length], id) {
            let name = name.trim_end_matches('.');
            return name
                .split('.')
                .next()
                .filter(|label| !label.is_empty())
                .map(str::to_string);
        }
    }
}

/// A PTR question for d.c.b.a.in-addr.arpa, recursion desired
pub fn ptr_query(address: Ipv4Addr, id: u16) -> Vec<u8> {
    let mut packet = Vec::with_capacity(48);
    packet.extend_from_slice(&id.to_be_bytes());
    packet.extend_from_slice(&[0x01, 0x00, 0, 1, 0, 0, 0, 0, 0, 0]);
    let o = address.octets();
    for label in [
        o[3].to_string(),
        o[2].to_string(),
        o[1].to_string(),
        o[0].to_string(),
        "in-addr".into(),
        "arpa".into(),
    ] {
        packet.push(label.len() as u8);
        packet.extend_from_slice(label.as_bytes());
    }
    packet.extend_from_slice(&[0, 0, 12, 0, 1]);
    packet
}

fn u16_at(message: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes([
        *message.get(at)?,
        *message.get(at + 1)?,
    ]))
}

/// The name in the first PTR answer of a reply to query `id`. None for errors, other record types
/// and anything malformed
pub fn parse_ptr_response(message: &[u8], id: u16) -> Option<String> {
    let flags = u16_at(message, 2)?;
    if u16_at(message, 0)? != id || flags & 0x8000 == 0 || flags & 0xf != 0 {
        return None;
    }
    let questions = u16_at(message, 4)?;
    let answers = u16_at(message, 6)?;
    let mut at = 12;
    for _ in 0..questions {
        at = read_name(message, at)?.1 + 4;
    }
    for _ in 0..answers {
        let (_, end) = read_name(message, at)?;
        let kind = u16_at(message, end)?;
        let class = u16_at(message, end + 2)?;
        let length = u16_at(message, end + 8)? as usize;
        let data = end + 10;
        if data + length > message.len() {
            return None;
        }
        if kind == 12 && class == 1 {
            return read_name(message, data)
                .map(|(name, _)| name)
                .filter(|n| !n.is_empty());
        }
        at = data + length;
    }
    None
}

/// Reads a (possibly compressed) DNS name as (name, offset after it). Labels must be letters,
/// digits, '-' or '_', which also keeps terminal escapes out of the output
pub fn read_name(message: &[u8], start: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut at = start;
    let mut end = None;
    let mut total = 0;
    for _ in 0..128 {
        let length = *message.get(at)? as usize;
        match length {
            0 => {
                return Some((labels.join("."), end.unwrap_or(at + 1)));
            },
            l if l & 0xc0 == 0xc0 => {
                let pointer = (u16_at(message, at)? & 0x3fff) as usize;
                end.get_or_insert(at + 2);
                if labels.len() > 127 || pointer >= message.len() {
                    return None;
                }
                at = pointer;
            },
            l if l & 0xc0 != 0 => return None,
            l => {
                let label = message.get(at + 1..at + 1 + l)?;
                if !label
                    .iter()
                    .all(|b| b.is_ascii_alphanumeric() || *b == b'-' || *b == b'_')
                {
                    return None;
                }
                total += l + 1;
                if total > 255 {
                    return None;
                }
                labels.push(String::from_utf8_lossy(label).into_owned());
                at += 1 + l;
            },
        }
    }
    // a pointer loop
    None
}

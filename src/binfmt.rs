//! Reads just the C string literals of an executable (Mach-O `__TEXT,__cstring`, ELF `.rodata`),
//! where shells keep their version string. That's ~25 KB of zsh's 1.3 MB binary, so finding a
//! version costs a couple of small reads instead of a whole file or a process.

use std::{fs::File, os::unix::fs::FileExt};

/// A section bigger than this means a corrupt file
const MAX_SECTION: u64 = 8 * 1024 * 1024;
const HEADER_READ: usize = 32 * 1024;

pub fn string_section(path: &str) -> Option<Vec<u8>> {
    let file = File::open(path).ok()?;
    let head = read_at(&file, 0, HEADER_READ)?;
    let (offset, size) = match head.get(..4)? {
        [0x7f, b'E', b'L', b'F'] => elf_rodata(&file, &head)?,
        _ => macho_cstring(&file, &head)?,
    };
    (size <= MAX_SECTION).then_some(())?;
    read_at(&file, offset, size as usize)
}

fn read_at(file: &File, offset: u64, length: usize) -> Option<Vec<u8>> {
    let mut buffer = vec![0u8; length];
    let mut filled = 0;
    while filled < length {
        match file.read_at(&mut buffer[filled..], offset + filled as u64) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(_) => return None,
        }
    }
    buffer.truncate(filled);
    Some(buffer)
}

fn u16le(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at + 2)?.try_into().ok()?))
}
fn u32le(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at + 4)?.try_into().ok()?))
}
fn u64le(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(d.get(at..at + 8)?.try_into().ok()?))
}
fn u32be(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(at..at + 4)?.try_into().ok()?))
}
fn u64be(d: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_be_bytes(d.get(at..at + 8)?.try_into().ok()?))
}

/// Finds `__TEXT,__cstring` in a 64-bit Mach-O, or in the first slice of a universal binary
/// (every slice carries the same strings)
fn macho_cstring(file: &File, head: &[u8]) -> Option<(u64, u64)> {
    let (slice, header) = match u32be(head, 0)? {
        0xcafebabe => {
            let offset = u32be(head, 16)? as u64;
            (offset, read_at(file, offset, HEADER_READ)?)
        },
        0xcafebabf => {
            let offset = u64be(head, 16)?;
            (offset, read_at(file, offset, HEADER_READ)?)
        },
        _ => (0, head.to_vec()),
    };
    if u32le(&header, 0)? != 0xfeedfacf {
        return None;
    }
    let commands = u32le(&header, 16)?;
    let mut at = 32;
    for _ in 0..commands {
        let cmd = u32le(&header, at)?;
        let size = u32le(&header, at + 4)? as usize;
        if size == 0 {
            return None;
        }
        // LC_SEGMENT_64 named __TEXT
        if cmd == 0x19 && header.get(at + 8..at + 24)?.starts_with(b"__TEXT\0") {
            let sections = u32le(&header, at + 64)? as usize;
            for i in 0..sections {
                let section = at + 72 + i * 80;
                if header
                    .get(section..section + 16)?
                    .starts_with(b"__cstring\0")
                {
                    let size = u64le(&header, section + 40)?;
                    let offset = u32le(&header, section + 48)? as u64;
                    return Some((slice + offset, size));
                }
            }
        }
        at = at.checked_add(size)?;
    }
    None
}

/// Finds `.rodata` in a little-endian ELF, 32 or 64-bit (every target haikufetch runs on is LE)
fn elf_rodata(file: &File, head: &[u8]) -> Option<(u64, u64)> {
    if *head.get(5)? != 1 {
        return None;
    }
    let wide = *head.get(4)? == 2;
    let (table, entry_size, count, names_index) = if wide {
        (
            u64le(head, 0x28)?,
            u16le(head, 0x3a)?,
            u16le(head, 0x3c)?,
            u16le(head, 0x3e)?,
        )
    } else {
        (
            u32le(head, 0x20)? as u64,
            u16le(head, 0x2e)?,
            u16le(head, 0x30)?,
            u16le(head, 0x32)?,
        )
    };
    let headers = read_at(
        file,
        table,
        (entry_size as usize).checked_mul(count as usize)?,
    )?;
    let section = |index: usize| -> Option<(u32, u64, u64)> {
        let at = index * entry_size as usize;
        let name = u32le(&headers, at)?;
        if wide {
            Some((name, u64le(&headers, at + 24)?, u64le(&headers, at + 32)?))
        } else {
            Some((
                name,
                u32le(&headers, at + 16)? as u64,
                u32le(&headers, at + 20)? as u64,
            ))
        }
    };
    let (_, names_offset, names_size) = section(names_index as usize)?;
    if names_size > MAX_SECTION {
        return None;
    }
    let names = read_at(file, names_offset, names_size as usize)?;
    (0..count as usize).find_map(|i| {
        let (name, offset, size) = section(i)?;
        names
            .get(name as usize..)?
            .starts_with(b".rodata\0")
            .then_some((offset, size))
    })
}

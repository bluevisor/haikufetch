//! Pure formatting helpers: sizes, durations, frequencies and usage bars. No I/O, no color, so
//! every rule here is pinned down by unit tests.

/// Formats a byte count in binary units, with fewer decimals as the number grows so a line stays
/// short: "512 MiB", "3.53 GiB", "50.2 GiB", "803 GiB". Trailing zeros are dropped ("64 GiB")
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        return format!("{bytes} B");
    }
    format!("{} {}", decimals(value), UNITS[unit])
}

/// Prints a number with 2 decimals under 10, 1 under 100 and none above, without trailing zeros.
/// Rounds down so a nearly full disk never reads as its total
fn decimals(value: f64) -> String {
    let places: i32 = if value < 10.0 {
        2
    } else if value < 100.0 {
        1
    } else {
        0
    };
    let scale = 10f64.powi(places);
    let truncated = (value * scale).floor() / scale;
    let text = format!("{:.*}", places as usize, truncated);
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        text
    }
}

/// Formats an uptime as days, hours and minutes: "2d 2h 46m", "5h 3m", "12m". Under a minute
/// shows seconds, since "0m" right after boot looks broken
pub fn duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3600;
    let minutes = seconds % 3600 / 60;
    match (days, hours) {
        (0, 0) if minutes == 0 => format!("{seconds}s"),
        (0, 0) => format!("{minutes}m"),
        (0, _) => format!("{hours}h {minutes}m"),
        _ => format!("{days}d {hours}h {minutes}m"),
    }
}

/// Formats a frequency given in MHz like fastfetch: "4.61 GHz", "800 MHz". None for 0, which
/// means it couldn't be read
pub fn frequency(mhz: u64) -> Option<String> {
    match mhz {
        0 => None,
        1..1000 => Some(format!("{mhz} MHz")),
        _ => Some(format!("{:.2} GHz", mhz as f64 / 1000.0)),
    }
}

/// Floors a ratio into a whole percentage, None when the total is 0
pub fn percent(used: u64, total: u64) -> Option<u8> {
    if total == 0 {
        return None;
    }
    Some(((used as u128 * 100) / total as u128).min(100) as u8)
}

/// Eighth blocks that end a bar's filled part, from 1/8 to 7/8 of a cell
const PARTIAL: [char; 7] = ['▏', '▎', '▍', '▌', '▋', '▊', '▉'];

/// Draws a usage bar `width` cells wide as its (filled, empty) parts so each can get its own
/// color. The filled part is rounded to the nearest eighth of a cell: (64, 10) gives
/// ("██████▍", "░░░")
pub fn bar(percent: u8, width: usize) -> (String, String) {
    let eighths = (percent.min(100) as usize * width * 8 + 50) / 100;
    let mut filled = "█".repeat(eighths / 8);
    if let Some(&block) = (eighths % 8).checked_sub(1).and_then(|i| PARTIAL.get(i)) {
        filled.push(block);
    }
    let empty = "░".repeat(width.saturating_sub(filled.chars().count()));
    (filled, empty)
}

/// Formats core counts as "P+E" for hybrid CPUs and a plain count otherwise. None when a count is
/// missing, since "6+0" would be wrong
pub fn cores(counts: &[u32]) -> Option<String> {
    if counts.is_empty() || counts.contains(&0) {
        return None;
    }
    Some(
        counts
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join("+"),
    )
}

/// Strips the marketing noise CPUs carry in their brand string: "AMD Ryzen 5 5600X 6-Core
/// Processor" gives "AMD Ryzen 5 5600X", "Intel(R) Core(TM) i7-8700K CPU @ 3.70GHz" gives
/// "Intel Core i7-8700K"
pub fn clean_cpu_name(brand: &str) -> String {
    let mut name = brand
        .replace("(R)", "")
        .replace("(TM)", "")
        .replace("(tm)", "");
    for cut in [" @ ", " with ", " CPU", " Processor"] {
        if let Some(pos) = name.find(cut) {
            name.truncate(pos);
        }
    }
    // "6-Core", "Eight-Core": the core count is shown separately
    if let Some(pos) = name.find("-Core")
        && let Some(space) = name[..pos].rfind(' ')
    {
        name.truncate(space);
    }
    name.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escapes a string for a JSON document
pub fn json_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

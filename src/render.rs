//! Puts the logo and the lines side by side, or prints JSON.

use std::{io::Write, sync::OnceLock};

use crate::{
    format,
    logo::Rendered,
    modules::Output,
    style::{Color, Painter},
};

static KEY_COLOR: OnceLock<Option<Color>> = OnceLock::new();

/// Set once before the modules run: the color of every label (None: plain bold)
pub fn set_key_color(color: Option<Color>) {
    let _ = KEY_COLOR.set(color);
}

pub fn key_color() -> Option<Color> {
    KEY_COLOR.get().copied().flatten()
}

/// "OS:" in bold, the label in the key color and the colon in the text color
pub fn key(painter: &Painter, label: &str) -> String {
    let mut out = String::new();
    painter.push_bold(&mut out);
    if let Some(color) = key_color() {
        painter.push_fg(&mut out, color);
        out.push_str(label);
        painter.push_reset(&mut out);
        painter.push_bold(&mut out);
    } else {
        out.push_str(label);
    }
    out.push(':');
    painter.push_reset(&mut out);
    out
}

/// The text lines: "Key: value", or the value alone for keyless entries
pub fn lines(outputs: &[Output], painter: &Painter) -> Vec<String> {
    outputs
        .iter()
        .flat_map(|o| &o.entries)
        .map(|e| {
            if e.key.is_empty() {
                e.value.clone()
            } else {
                format!("{} {}", key(painter, &e.key), e.value)
            }
        })
        .collect()
}

/// Writes the logo with the lines to its right. Rows past the logo are indented to line up
pub fn print(logo: Option<&Rendered>, lines: &[String], padding: usize) -> std::io::Result<()> {
    let mut out = String::new();
    match logo {
        None => {
            for line in lines {
                out.push_str(line);
                out.push('\n');
            }
        },
        Some(logo) => {
            let rows = logo.lines.len().max(lines.len());
            for row in 0..rows {
                let art = logo.lines.get(row).map(String::as_str).unwrap_or("");
                out.push_str(art);
                let text = lines.get(row).map(String::as_str).unwrap_or("");
                if !text.is_empty() {
                    let gap =
                        logo.width - crate::style::visible_width(art).min(logo.width) + padding;
                    out.extend(std::iter::repeat_n(' ', gap));
                    out.push_str(text);
                }
                out.push('\n');
            }
        },
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(out.as_bytes())?;
    stdout.flush()
}

/// One JSON object, module name to its facts (an array for modules with several lines)
pub fn json(outputs: &[Output]) -> String {
    let mut fields = Vec::new();
    for output in outputs {
        let values: Vec<&str> = output
            .entries
            .iter()
            .map(|e| e.json.as_str())
            .filter(|j| *j != "null")
            .collect();
        let value = match values.as_slice() {
            [] => continue,
            [one] if !matches!(output.module.name, "screen" | "gpu" | "disk") => one.to_string(),
            many => format!("[{}]", many.join(",")),
        };
        fields.push(format!(
            "  {}: {}",
            format::json_string(output.module.name),
            value
        ));
    }
    format!("{{\n{}\n}}\n", fields.join(",\n"))
}

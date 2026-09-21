//! Human output: aligned tables, key/value blocks, and colored JSON.

use std::io::IsTerminal;

pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Self {
        Self {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub fn print(&self) {
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| h.chars().count()).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().enumerate().take(cols) {
                widths[i] = widths[i].max(cell.chars().count());
            }
        }
        let line = |cells: &[String]| {
            let mut out = String::new();
            for (i, cell) in cells.iter().enumerate().take(cols) {
                if i > 0 {
                    out.push_str("  ");
                }
                if i + 1 == cols {
                    out.push_str(cell);
                } else {
                    out.push_str(&format!("{:<width$}", cell, width = widths[i]));
                }
            }
            out
        };
        println!("{}", line(&self.headers));
        for row in &self.rows {
            println!("{}", line(row));
        }
    }
}

pub fn kv(pairs: &[(&str, String)]) {
    let width = pairs.iter().map(|(k, _)| k.len()).max().unwrap_or(0);
    for (k, v) in pairs {
        println!("{:<width$}  {}", k, v, width = width);
    }
}

pub fn fmt_ms(ms: u64) -> String {
    let secs = ms / 1000;
    if secs < 60 {
        format!("{}.{:01}s", secs, (ms % 1000) / 100)
    } else if secs < 3600 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{}h{:02}m", secs / 3600, (secs % 3600) / 60)
    }
}

pub fn yes_no(flag: bool) -> &'static str {
    if flag { "yes" } else { "no" }
}

/// Only when a human is actually watching: never on a pipe, and never with
/// `NO_COLOR` set, so nothing downstream (a script, `| jq`, a log file)
/// ever has to see an escape code.
fn use_color() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

const RESET: &str = "\x1b[0m";
const PUNCT: &str = "\x1b[2m";
const KEY: &str = "\x1b[1;34m";
const STRING: &str = "\x1b[32m";
const NUMBER: &str = "\x1b[36m";
const BOOLEAN: &str = "\x1b[33m";
const NULL: &str = "\x1b[90m";

/// Pretty-printed JSON, colored jq-style when a terminal is actually
/// showing it to someone, plain otherwise.
pub fn pretty_json(value: &serde_json::Value) -> String {
    if !use_color() {
        return serde_json::to_string_pretty(value).unwrap_or_default();
    }
    let mut text = String::new();
    write_colored(value, 0, &mut text);
    text
}

fn write_colored(value: &serde_json::Value, indent: usize, out: &mut String) {
    use serde_json::Value;
    let pad = |n: usize| "  ".repeat(n);
    match value {
        Value::Null => out.push_str(&format!("{NULL}null{RESET}")),
        Value::Bool(b) => out.push_str(&format!("{BOOLEAN}{b}{RESET}")),
        Value::Number(n) => out.push_str(&format!("{NUMBER}{n}{RESET}")),
        Value::String(s) => {
            out.push_str(&format!(
                "{STRING}{}{RESET}",
                serde_json::to_string(s).unwrap_or_default()
            ));
        }
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str(&format!("{PUNCT}[]{RESET}"));
                return;
            }
            out.push_str(&format!("{PUNCT}[{RESET}\n"));
            for (i, item) in items.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                write_colored(item, indent + 1, out);
                if i + 1 < items.len() {
                    out.push_str(&format!("{PUNCT},{RESET}"));
                }
                out.push('\n');
            }
            out.push_str(&pad(indent));
            out.push_str(&format!("{PUNCT}]{RESET}"));
        }
        Value::Object(map) => {
            if map.is_empty() {
                out.push_str(&format!("{PUNCT}{{}}{RESET}"));
                return;
            }
            out.push_str(&format!("{PUNCT}{{{RESET}\n"));
            let len = map.len();
            for (i, (k, v)) in map.iter().enumerate() {
                out.push_str(&pad(indent + 1));
                out.push_str(&format!(
                    "{KEY}{}{RESET}{PUNCT}:{RESET} ",
                    serde_json::to_string(k).unwrap_or_default()
                ));
                write_colored(v, indent + 1, out);
                if i + 1 < len {
                    out.push_str(&format!("{PUNCT},{RESET}"));
                }
                out.push('\n');
            }
            out.push_str(&pad(indent));
            out.push_str(&format!("{PUNCT}}}{RESET}"));
        }
    }
}

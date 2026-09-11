//! Human output: aligned tables and key/value blocks.

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

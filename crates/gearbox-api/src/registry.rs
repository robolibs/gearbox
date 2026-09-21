//! Host-local instance registry: one JSON file per running host under
//! `$XDG_RUNTIME_DIR/gearbox/`, pruned when its pid is gone.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RegistryEntry {
    pub name: String,
    pub did: String,
    pub addr: String,
    pub pid: u32,
    pub started: String,
    pub version: String,
    #[serde(default)]
    pub log: Option<String>,
}

pub fn registry_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("GEARBOX_REGISTRY_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join(format!("gearbox-{}", whoami())));
    base.join("gearbox")
}

fn whoami() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".to_string())
}

pub fn entry_path(name: &str) -> PathBuf {
    registry_dir().join(format!("{}.json", sanitize(name)))
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn write(entry: &RegistryEntry) -> std::io::Result<PathBuf> {
    let path = entry_path(&entry.name);
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_string_pretty(entry).map_err(std::io::Error::other)?;
    fs::write(&path, json)?;
    Ok(path)
}

pub fn remove(name: &str) {
    let _ = fs::remove_file(entry_path(name));
}

pub fn read(path: &Path) -> Option<RegistryEntry> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

/// Live entries. Stale files whose pid is gone are deleted on the way.
pub fn list() -> Vec<RegistryEntry> {
    let Ok(dir) = fs::read_dir(registry_dir()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in dir.flatten() {
        let path = item.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        match read(&path) {
            Some(entry) if pid_alive(entry.pid) => out.push(entry),
            _ => {
                let _ = fs::remove_file(&path);
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn find(name_or_did: &str) -> Option<RegistryEntry> {
    list()
        .into_iter()
        .find(|e| e.name == name_or_did || e.did == name_or_did)
}

pub fn pid_alive(pid: u32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

pub fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Peerbus's Python binding addresses peers by this form: the postcard
/// encoding of the endpoint address, as lowercase hex.
pub fn endpoint_addr_hex(addr: &peerbus::EndpointAddr) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let bytes = postcard::to_stdvec(addr).unwrap_or_default();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in &bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

//! Shared wire pieces: status codes, the string-map payload, decoding.

use datapod::{DataPodDecode, Map, WireError};

/// Prop carrying the sender's wall clock in unix milliseconds; a host drops
/// load and delete requests stamped before it started.
pub const SENT_AT: &str = "sent_at";

pub fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub mod code {
    pub const OK: u32 = 0;
    pub const ERROR: u32 = 1;
    pub const USAGE: u32 = 2;
    pub const NO_INSTANCE: u32 = 3;
    pub const REFUSED: u32 = 4;
    pub const UNSUPPORTED: u32 = 5;
    pub const BUSY: u32 = 6;
    pub const TIMEOUT: u32 = 7;
    pub const NOT_FOUND: u32 = 8;
}

#[datapod::datapod(name = "gearbox.ping.v1")]
#[derive(Default)]
pub struct Ping {
    pub nonce: u64,
}

#[datapod::datapod(name = "gearbox.status.v1")]
#[derive(Default)]
pub struct Status {
    pub code: u32,
    #[dp(bytes)]
    pub detail: Vec<u8>,
}

impl Status {
    pub fn ok() -> Self {
        Self::with(code::OK, &[])
    }

    pub fn ok_with(props: &Props) -> Self {
        Self::with(code::OK, &[]).merge(props)
    }

    pub fn err(code: u32, message: &str) -> Self {
        Self::with(code, &[("message", message)])
    }

    pub fn with(code: u32, pairs: &[(&str, &str)]) -> Self {
        let mut props = Props::new();
        for (k, v) in pairs {
            props.set(k, v);
        }
        Self {
            code,
            detail: props.into_bytes(),
        }
    }

    fn merge(mut self, props: &Props) -> Self {
        let mut own = Props::from_bytes(&self.detail);
        for (k, v) in props.iter() {
            own.set(&k, &v);
        }
        self.detail = own.into_bytes();
        self
    }

    pub fn is_ok(&self) -> bool {
        self.code == code::OK
    }

    pub fn props(&self) -> Props {
        Props::from_bytes(&self.detail)
    }

    pub fn message(&self) -> String {
        self.props().get("message").unwrap_or_default()
    }
}

/// Owned string map that rides in a `#[dp(bytes)]` field.
#[derive(Debug, Clone, Default)]
pub struct Props(Map);

impl Props {
    pub fn new() -> Self {
        Self(Map::new())
    }

    pub fn from_bytes(bytes: &[u8]) -> Self {
        if bytes.len() < 4 {
            return Self::new();
        }
        Self(Map {
            data: bytes.to_vec(),
        })
    }

    pub fn from_pairs(pairs: &[(&str, &str)]) -> Self {
        Self::from_entries(pairs.iter().copied())
    }

    /// The map of `entries`, encoded in one pass; of two with one key the later wins.
    pub fn from_entries<K: AsRef<str>, V: AsRef<str>>(entries: impl IntoIterator<Item = (K, V)>) -> Self {
        let mut sorted: Vec<(K, V)> = entries.into_iter().collect();
        sorted.reverse();
        sorted.sort_by(|a, b| a.0.as_ref().cmp(b.0.as_ref()));
        sorted.dedup_by(|later, kept| later.0.as_ref() == kept.0.as_ref());
        let blob: usize = sorted.iter().map(|(k, v)| k.as_ref().len() + v.as_ref().len()).sum();
        let mut table = Vec::with_capacity(4 + sorted.len() * std::mem::size_of::<datapod::MapEntry>());
        let mut bytes = Vec::with_capacity(blob);
        table.extend_from_slice(&(sorted.len() as u32).to_le_bytes());
        for (k, v) in &sorted {
            let (k, v) = (k.as_ref().as_bytes(), v.as_ref().as_bytes());
            for field in [bytes.len(), k.len(), bytes.len() + k.len(), v.len()] {
                table.extend_from_slice(&(field as u32).to_le_bytes());
            }
            bytes.extend_from_slice(k);
            bytes.extend_from_slice(v);
        }
        table.extend_from_slice(&bytes);
        Self(Map { data: table })
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.0
            .try_get_str(key)
            .ok()
            .flatten()
            .map(|s| s.to_string())
    }

    pub fn get_f64(&self, key: &str) -> Option<f64> {
        self.get(key)?.parse().ok()
    }

    pub fn set(&mut self, key: &str, value: &str) {
        let _ = self.0.try_insert_str(key, value);
    }

    pub fn with(mut self, key: &str, value: &str) -> Self {
        self.set(key, value);
        self
    }

    pub fn contains(&self, key: &str) -> bool {
        self.0.try_contains_key(key.as_bytes()).unwrap_or(false)
    }

    pub fn iter(&self) -> Vec<(String, String)> {
        let n = self.0.try_size().unwrap_or(0);
        (0..n)
            .filter_map(|i| {
                let k = std::str::from_utf8(self.0.try_key_at(i).ok()?).ok()?;
                let v = std::str::from_utf8(self.0.try_value_at(i).ok()?).ok()?;
                Some((k.to_string(), v.to_string()))
            })
            .collect()
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0.data
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.data.clone()
    }
}

/// Rebuild an owned datapod from a received header and payload.
pub fn decode<T>(header: T::Header, payload: &[u8]) -> Result<T, WireError>
where
    T: DataPodDecode,
{
    T::from_wire_parts(header, payload.to_vec())
}

/// Every topic carries this type-erased envelope, so a Python or C client
/// with the same canonical datapod name speaks to a Rust server unchanged.
pub type Env = peerbus::DatapodMsg;

pub fn pack<T>(value: &T) -> Env
where
    T: datapod::DataPod + datapod::DataPodValidate,
    T::Header: datapod::LeWireHeader,
{
    Env::from_datapod(value)
}

pub fn unpack<T>(type_hash: u64, wire: &[u8]) -> Result<T, WireError>
where
    T: DataPodDecode + datapod::DataPodValidate,
    T::Header: datapod::LeWireHeader,
{
    Env::new(type_hash, wire.to_vec()).to_datapod::<T>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn props_from_entries_encode_as_inserting_them_one_by_one_does() {
        let entries = [("tools", "a,b"), ("machine_id", "eve"), ("link.arm.position", "0.25"), ("tools", "c"), ("", "")];
        let mut inserted = Map::new();
        for (k, v) in entries {
            inserted.insert_str(k, v);
        }
        let props = Props::from_entries(entries);
        assert_eq!(props.to_bytes(), inserted.data);
        assert_eq!(props.get("tools").as_deref(), Some("c"));
        assert_eq!(Props::from_bytes(&props.to_bytes()).iter().len(), 4);
    }
}

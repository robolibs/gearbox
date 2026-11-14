use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetRevision {
    pub incarnation: u64,
    pub generation: u64,
}

impl DatasetRevision {
    pub fn validate(&self) -> Result<(), String> {
        if self.incarnation == 0 || self.generation == 0 {
            return Err("dataset incarnation and generation must be nonzero".into());
        }
        Ok(())
    }
}

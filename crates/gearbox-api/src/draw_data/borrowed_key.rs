use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::BTreeMap;

use crate::draw::DrawKey;

trait Parts {
    fn parts(&self) -> (&str, &str);
}

impl Parts for DrawKey {
    fn parts(&self) -> (&str, &str) {
        (&self.0, &self.1)
    }
}

impl Parts for (&str, &str) {
    fn parts(&self) -> (&str, &str) {
        *self
    }
}

impl<'a> Borrow<dyn Parts + 'a> for DrawKey {
    fn borrow(&self) -> &(dyn Parts + 'a) {
        self
    }
}

impl PartialEq for dyn Parts + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for dyn Parts + '_ {}

impl PartialOrd for dyn Parts + '_ {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for dyn Parts + '_ {
    fn cmp(&self, other: &Self) -> Ordering {
        self.parts().cmp(&other.parts())
    }
}

pub(super) fn get<'a, T>(
    values: &'a BTreeMap<DrawKey, T>,
    namespace: &str,
    id: &str,
) -> Option<&'a T> {
    values.get(&(namespace, id) as &dyn Parts)
}

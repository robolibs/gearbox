//! Path-addressed stage queries. The prim readers in this crate were written
//! against stage-level helpers; the current openusd answers the same
//! questions through `Stage::prim(path)`, so this adapts one to the other.

use anyhow::Result;
use openusd::sdf;
use openusd::usd::Stage;

pub trait StageExt {
    fn api_schemas(&self, prim: &sdf::Path) -> Result<Vec<String>>;
    fn has_api_schema(&self, prim: &sdf::Path, name: &str) -> Result<bool>;
    fn type_name(&self, prim: &sdf::Path) -> Result<Option<String>>;
    fn prim_properties(&self, prim: &sdf::Path) -> Result<Vec<String>>;
    fn prim_children(&self, prim: &sdf::Path) -> Result<Vec<String>>;
}

/// The composed `default` value of an attribute, only when some layer
/// authored one: schema fallbacks (zero quaternions, infinite centres of
/// mass) are not opinions the physics readers can use.
pub fn authored_value(stage: &Stage, prim: &sdf::Path, name: &str) -> Option<sdf::Value> {
    let prim = stage.prim(prim.clone()).ok()?;
    let attr = prim.attribute(name);
    match attr.resolve_info().ok()?.source() {
        openusd::usd::ResolveInfoSource::Default
        | openusd::usd::ResolveInfoSource::TimeSamples
        | openusd::usd::ResolveInfoSource::ValueClips => attr.get::<sdf::Value>().ok().flatten(),
        _ => None,
    }
}

fn property_name(path: &sdf::Path) -> Option<String> {
    let s = path.as_str();
    let dot = s.rfind('.')?;
    Some(s[dot + 1..].to_string())
}

impl StageExt for Stage {
    fn api_schemas(&self, prim: &sdf::Path) -> Result<Vec<String>> {
        let prim = self.prim(prim.clone())?;
        Ok(prim
            .api_schemas()?
            .into_iter()
            .map(|t| t.as_str().to_string())
            .collect())
    }

    fn has_api_schema(&self, prim: &sdf::Path, name: &str) -> Result<bool> {
        Ok(self.prim(prim.clone())?.has_api_schema(name)?)
    }

    fn type_name(&self, prim: &sdf::Path) -> Result<Option<String>> {
        Ok(self
            .prim(prim)?
            .type_name()?
            .map(|t| t.as_str().to_string()))
    }

    fn prim_properties(&self, prim: &sdf::Path) -> Result<Vec<String>> {
        let prim = self.prim(prim.clone())?;
        let mut names: Vec<String> = prim
            .attributes()?
            .iter()
            .filter_map(|a| property_name(a.path()))
            .collect();
        names.extend(
            prim.relationships()?
                .iter()
                .filter_map(|r| property_name(r.path())),
        );
        Ok(names)
    }

    fn prim_children(&self, prim: &sdf::Path) -> Result<Vec<String>> {
        Ok(self
            .prim(prim)?
            .child_names()?
            .into_iter()
            .map(|t| t.as_str().to_string())
            .collect())
    }
}

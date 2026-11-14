//! One pass over the composed stage: every active prim with its type and
//! applied schemas, cached world matrices, and the composition facts the
//! compiler reports (instances, unloaded payloads, composition errors).

use std::cell::RefCell;
use std::collections::HashMap;

use bevy::math::{DMat4, DQuat};
use openusd::sdf::Path as SdfPath;
use openusd::usd::Stage;

use super::diagnostics::{Code, Diagnostic};
use super::model::{RigidTransform, StageUnits};
use super::xform::{self, Decomposed};
use crate::usd_ext::StageExt;

#[derive(Clone, Debug)]
pub struct PrimInfo {
    pub path: String,
    pub type_name: String,
    pub schemas: Vec<String>,
}

pub struct Scene<'a> {
    pub stage: &'a Stage,
    pub prims: Vec<PrimInfo>,
    pub index: HashMap<String, usize>,
    pub units: StageUnits,
    pub up: DQuat,
    pub diagnostics: Vec<Diagnostic>,
    world: RefCell<HashMap<String, (DMat4, Vec<String>)>>,
}

fn walk(stage: &Stage, path: SdfPath, out: &mut Vec<PrimInfo>, diagnostics: &mut Vec<Diagnostic>) {
    if path.as_str() != "/" {
        let Ok(prim) = stage.prim(&path) else { return };
        if !prim.is_active().unwrap_or(true) {
            return;
        }
        if prim.is_instance().unwrap_or(false) {
            diagnostics.push(Diagnostic::warning(
                Code::InstanceProxy,
                Some(path.as_str()),
                "instanceable prim: its prototype is not traversed, physics below it is not read",
            ));
        }
        if !prim.is_loaded().unwrap_or(true) {
            diagnostics.push(Diagnostic::warning(
                Code::PayloadUnloaded,
                Some(path.as_str()),
                "payload not loaded: prims below it are not read",
            ));
        }
        out.push(PrimInfo {
            path: path.as_str().to_string(),
            type_name: stage.type_name(&path).ok().flatten().unwrap_or_default(),
            schemas: stage.api_schemas(&path).unwrap_or_default(),
        });
    }
    for child in stage.prim_children(&path).unwrap_or_default() {
        if let Ok(child) = path.append_path(child.as_str()) {
            walk(stage, child, out, diagnostics);
        }
    }
}

impl<'a> Scene<'a> {
    pub fn new(stage: &'a Stage, scan_root: &str, units: StageUnits, mut diagnostics: Vec<Diagnostic>) -> Self {
        let mut prims = Vec::new();
        let root = openusd::sdf::path(scan_root).unwrap_or_else(|_| SdfPath::abs_root());
        walk(stage, root, &mut prims, &mut diagnostics);
        for error in stage.composition_errors() {
            diagnostics.push(Diagnostic::warning(Code::CompositionError, None, format!("{error:?}")));
        }
        let index = prims.iter().enumerate().map(|(i, p)| (p.path.clone(), i)).collect();
        let up = xform::stage_to_world(units.up_axis.value);
        Self { stage, prims, index, units, up, diagnostics, world: RefCell::new(HashMap::new()) }
    }

    pub fn info(&self, path: &str) -> Option<&PrimInfo> {
        self.index.get(path).map(|&i| &self.prims[i])
    }

    pub fn sdf(&self, path: &str) -> Option<SdfPath> {
        openusd::sdf::path(path).ok()
    }

    pub fn has_schema(&self, path: &str, name: &str) -> bool {
        self.info(path).is_some_and(|p| p.schemas.iter().any(|s| s == name))
    }

    /// Applied instances of a multiple-apply schema: `PhysicsDriveAPI:angular` → `angular`.
    pub fn schema_instances(&self, path: &str, name: &str) -> Vec<String> {
        let prefix = format!("{name}:");
        self.info(path)
            .map(|p| p.schemas.iter().filter_map(|s| s.strip_prefix(&prefix).map(str::to_string)).collect())
            .unwrap_or_default()
    }

    pub fn is_body(&self, path: &str) -> bool {
        self.has_schema(path, "PhysicsRigidBodyAPI")
    }

    pub fn is_joint(&self, path: &str) -> bool {
        self.info(path).is_some_and(|p| p.type_name.starts_with("Physics") && p.type_name.ends_with("Joint"))
    }

    pub fn exists(&self, path: &str) -> bool {
        self.index.contains_key(path)
    }

    /// Whether the composed stage has the prim at all, active or not.
    pub fn defined(&self, path: &str) -> bool {
        self.sdf(path)
            .and_then(|p| self.stage.prim(&p).ok())
            .is_some_and(|p| p.is_valid().unwrap_or(false))
    }

    pub fn ancestors(path: &str) -> impl Iterator<Item = &str> {
        let mut current = path;
        std::iter::from_fn(move || {
            let pos = current.rfind('/')?;
            if pos == 0 {
                return None;
            }
            current = &current[..pos];
            Some(current)
        })
    }

    pub fn is_under(path: &str, root: &str) -> bool {
        path == root || (path.starts_with(root) && path.as_bytes().get(root.len()) == Some(&b'/'))
    }

    pub fn nearest_body_ancestor(&self, path: &str) -> Option<String> {
        Self::ancestors(path).find(|a| self.is_body(a)).map(str::to_string)
    }

    /// Composed local-to-world matrix in stage axes and units.
    pub fn world_matrix(&self, path: &str) -> DMat4 {
        if let Some((m, _)) = self.world.borrow().get(path) {
            return *m;
        }
        let computed = xform::world_matrix(self.stage, path);
        let m = computed.0;
        self.world.borrow_mut().insert(path.to_string(), computed);
        m
    }

    pub fn unsupported_ops(&self, path: &str) -> Vec<String> {
        self.world_matrix(path);
        self.world.borrow().get(path).map(|(_, u)| u.clone()).unwrap_or_default()
    }

    pub fn decomposed(&self, path: &str) -> Decomposed {
        xform::decompose(&self.world_matrix(path))
    }

    /// World pose with scale stripped, in the normalized Z-up SI convention.
    pub fn rigid_world(&self, path: &str) -> RigidTransform {
        self.to_world(self.decomposed(path).rigid)
    }

    /// A stage-axes, stage-unit rigid transform in the normalized convention.
    pub fn to_world(&self, stage: RigidTransform) -> RigidTransform {
        RigidTransform::new(self.up * (stage.translation * self.units.l()), (self.up * stage.rotation).normalize())
    }
}

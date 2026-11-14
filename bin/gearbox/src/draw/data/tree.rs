use gearbox_api::draw_data::{CpuBudget, StorageError};

use super::cloud::Records;
use super::hierarchy::Hierarchy;
use super::refinement::LEVELS;

/// Parent of a page root.
pub(super) const ROOT: u32 = u32::MAX;

/// One hierarchy node as the GPU level-of-detail pass reads it: local bounds widened by the
/// page's position error, its range of the refinement order, and for leaves the sample
/// spacing of each power-of-two prefix.
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
pub(super) struct TreeNode {
    pub min: [f32; 3],
    pub first: u32,
    pub max: [f32; 3],
    pub count: u32,
    pub parent: u32,
    /// Refinement-order position of the point drawn while this node stands for its subtree.
    pub start: u32,
    pub leaf: u32,
    pub pad: u32,
    pub errors_m: [f32; LEVELS],
}

pub(super) fn build(
    budget: &CpuBudget,
    hierarchy: &Hierarchy,
    rebase_m: [f64; 3],
    max_error_m: f64,
) -> Result<Records<TreeNode>, StorageError> {
    let nodes = &hierarchy.nodes.values;
    let mut tree = Records::new(budget, nodes.len())?;
    let mut parents = Records::<u32>::new(budget, nodes.len())?;
    parents.values.fill(ROOT);
    for (index, node) in nodes.iter().enumerate() {
        for child in node.children.into_iter().flatten() {
            parents.values[child as usize] = index as u32;
        }
    }
    // Preorder puts children after their parent, so starts resolve from the back.
    let mut starts = Records::<u32>::new(budget, nodes.len())?;
    for (index, node) in nodes.iter().enumerate().rev() {
        starts.values[index] = match node.children {
            None => node.first,
            Some(children) => children
                .into_iter()
                .find(|child| nodes[*child as usize].representative == node.representative)
                .map_or(node.first, |child| starts.values[child as usize]),
        };
    }
    let (parents, starts) = (&parents.values, &starts.values);
    for (index, node) in nodes.iter().enumerate() {
        let side = |side: usize, axis: usize| {
            let local = node.bounds_m[side][axis] - rebase_m[axis];
            if side == 0 {
                ((local - max_error_m) as f32).next_down()
            } else {
                ((local + max_error_m) as f32).next_up()
            }
        };
        let errors = hierarchy.refinement.errors_m.values.get(index).copied().unwrap_or_default();
        tree.values[index] = TreeNode {
            min: std::array::from_fn(|axis| side(0, axis)),
            first: node.first,
            max: std::array::from_fn(|axis| side(1, axis)),
            count: node.count,
            parent: parents[index],
            start: starts[index],
            leaf: u32::from(node.children.is_none()),
            pad: 0,
            errors_m: errors.map(|error| (error as f32).next_up()),
        };
    }
    Ok(tree)
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;

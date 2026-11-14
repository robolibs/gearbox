use super::*;
use crate::draw::data::cloud::Geometry;
use crate::draw::data::tests::{fixture, stored};

#[test]
fn tree_nodes_link_parents_starts_and_bounds_of_every_point() {
    let budget = CpuBudget::new(32 << 20);
    let count = 3001;
    let points: Vec<_> = (0..count)
        .map(|i| [1e6 + (i % 61) as f64 * 0.37, (i % 89) as f64 * 0.21, (i % 13) as f64 * 1.3])
        .collect();
    let (schema, page) = fixture(&points, vec![]);
    let geometry =
        Geometry::prepare(&budget, &schema, stored(&budget, &schema, &page, [0.0; 3])).unwrap();
    let nodes = &geometry.hierarchy.nodes.values;
    let tree = &geometry.tree.values;
    let order = &geometry.hierarchy.refinement.order.values;
    assert_eq!(tree.len(), nodes.len());
    assert_eq!(size_of::<TreeNode>(), 80);
    assert_eq!(tree[0].parent, ROOT);
    for (index, (node, record)) in nodes.iter().zip(tree).enumerate() {
        assert_eq!((record.first, record.count), (node.first, node.count));
        assert_eq!(record.leaf == 1, node.children.is_none());
        assert_eq!(order[record.start as usize], node.representative, "node {index}");
        assert!((node.first..node.first + node.count).contains(&record.start));
        for child in node.children.into_iter().flatten() {
            assert_eq!(tree[child as usize].parent, index as u32);
        }
        for &ordinal in &order[node.first as usize..(node.first + node.count) as usize] {
            let xyz = geometry.positions.values[ordinal as usize].xyz;
            for axis in 0..3 {
                assert!(record.min[axis] <= xyz[axis] && xyz[axis] <= record.max[axis]);
            }
        }
        if record.leaf == 1 {
            assert!(record.errors_m.windows(2).all(|pair| pair[1] <= pair[0]));
        }
    }
    drop(geometry);
    assert_eq!(budget.usage().committed(), 0);
}

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use gearbox_api::draw_data::{Charge, CpuBudget, MemoryClass};

use super::*;

struct Owner {
    live: Arc<AtomicBool>,
    _charge: Charge,
}

impl Live for Owner {
    fn live(&self) -> bool {
        self.live.load(Ordering::Acquire)
    }
}

#[test]
fn retirement_preserves_readers_and_live_neighbors_without_allocating() {
    let budget = CpuBudget::new(16 << 10);
    let states = std::array::from_fn::<_, 3, _>(|_| Arc::new(AtomicBool::new(true)));
    let mut list = List::new(&budget, states.len(), MemoryClass::Metadata).unwrap();
    for live in &states {
        list.push(Owner {
            live: live.clone(),
            _charge: budget
                .reserve(128)
                .unwrap()
                .charge(128, MemoryClass::Page)
                .unwrap(),
        })
        .unwrap();
    }
    let list = Arc::new(LiveList::from(list));
    let before = budget.usage();
    let pressure = budget.reserve(before.limit - before.committed()).unwrap();
    states[0].store(false, Ordering::Release);
    let reader = list.read();
    assert_eq!(list.prune(), 0);
    assert_eq!(reader.len(), 3);
    assert_eq!(budget.usage().allocated, before.allocated);
    drop(reader);
    assert_eq!(list.prune(), 1);
    assert_eq!(list.len(), 2);
    assert_eq!(budget.usage().allocated, before.allocated - 128);
    assert!(Arc::ptr_eq(&list.read()[0].live, &states[1]));
    for live in &states {
        live.store(false, Ordering::Release);
    }
    assert_eq!(list.prune(), 2);
    assert!(list.is_empty());
    assert_eq!(budget.usage().allocated, 0);
    assert_eq!(budget.usage().peak, before.limit);
    drop(pressure);
    assert_eq!(budget.usage().committed(), 0);
}

#[test]
fn empty_shared_lists_release_unused_backing_capacity() {
    let budget = CpuBudget::new(4096);
    let values = List::<Owner>::new(&budget, 8, MemoryClass::Metadata).unwrap();
    assert!(budget.usage().allocated > 0);
    let list = LiveList::from(values);
    assert!(list.is_empty());
    assert_eq!(budget.usage().allocated, 0);
}

#[test]
fn visitor_marking_preserves_keys_and_cleans_up_without_headroom() {
    let budget = CpuBudget::new(16 << 10);
    let mut map = super::super::owned::Map::default();
    for key in 0..4 {
        map.insert(&budget, key, key).unwrap();
    }
    let before = budget.usage();
    let pressure = budget.reserve(before.limit - before.committed()).unwrap();
    let mut removed = 0;
    map.retain_keys_by(
        |mark| {
            mark(&1);
            mark(&1);
            mark(&3);
            mark(&99);
        },
        |_, _| removed += 1,
    );
    assert_eq!(removed, 2);
    assert!(map.contains_key(&1) && map.contains_key(&3));
    assert_eq!(budget.usage().allocated, before.allocated);
    map.retain_keys_by(|_| {}, |_, _| removed += 1);
    assert_eq!(removed, 4);
    assert_eq!(budget.usage().allocated, 0);
    drop(pressure);
}

#[test]
fn failure_text_is_a_bounded_utf8_prefix_across_format_arguments() {
    assert_eq!(Text::new(&"short reason").to_string(), "short reason");
    let prefix = "a".repeat(509);
    let text = Text::new(&format_args!("{prefix}{}{}", "🙂", "ignored suffix"));
    assert_eq!(text.to_string(), prefix);
    let text = Text::new(&"é".repeat(300));
    assert_eq!(text.to_string(), "é".repeat(256));
}

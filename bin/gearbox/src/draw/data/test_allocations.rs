use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    static ACTIVE: Cell<Option<usize>> = const { Cell::new(None) };
}

struct Allocator;

#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

fn allocation() {
    let _ = ACTIVE.try_with(|active| {
        if let Some(count) = active.get() {
            active.set(Some(count + 1));
        }
    });
}

unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        allocation();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        allocation();
        unsafe { System.realloc(pointer, layout, size) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }
}

pub(super) fn count<T>(run: impl FnOnce() -> T) -> (T, usize) {
    struct Reset;
    impl Drop for Reset {
        fn drop(&mut self) {
            ACTIVE.with(|active| active.set(None));
        }
    }

    ACTIVE.with(|active| {
        assert!(active.get().is_none(), "nested allocation measurement");
        active.set(Some(0));
    });
    let reset = Reset;
    let result = run();
    let count = ACTIVE.with(|active| active.get().unwrap());
    drop(reset);
    (result, count)
}

#[test]
fn counter_observes_real_allocations_and_recovers_after_unwinding() {
    let (values, allocations) = count(|| std::hint::black_box(vec![0u8; 1024]));
    assert_eq!(values.len(), 1024);
    assert!(allocations > 0);
    assert!(std::panic::catch_unwind(|| count(|| panic!("scope cleanup"))).is_err());
    assert_eq!(count(|| 7), (7, 0));
}

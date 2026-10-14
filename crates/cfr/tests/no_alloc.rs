//! Separate test binary avoids concurrent tests contaminating global allocation counts.
use cards::{Card, Combo};
use cfr::{Engine, EngineConfig};
use ranges::Range;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tree::{GameTree, TreeConfig};
struct Counter;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        if ACTIVE.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        unsafe { System.realloc(p, layout, size) }
    }
}
#[global_allocator]
static ALLOCATOR: Counter = Counter;
#[test]
fn warm_serial_and_parallel_iterations_allocate_nothing() {
    for threads in [1, 2] {
        let board: Vec<_> = "2c 7d 9h Js"
            .split_whitespace()
            .map(|c| c.parse::<Card>().unwrap())
            .collect();
        let rr = ["AcAd", "KcKd"].map(|c| {
            Range::from_combos(vec![(c.parse::<Combo>().unwrap(), 1.0)])
                .unwrap()
                .blocked(&board)
                .unwrap()
        });
        let tree = GameTree::build(
            board.iter().map(|c| c.0).collect(),
            TreeConfig {
                starting_pot: 1000,
                effective_stack: 500,
                allow_raises: false,
                ..Default::default()
            },
        )
        .unwrap();
        let mut engine = Engine::new(
            tree,
            rr,
            EngineConfig {
                threads,
                ..Default::default()
            },
        )
        .unwrap();
        engine.with_pool(|engine| {
            let _ = engine.metrics();
            ALLOCS.store(0, Ordering::Relaxed);
            BYTES.store(0, Ordering::Relaxed);
            ACTIVE.store(true, Ordering::SeqCst);
            for _ in 0..32 {
                engine.iterate();
            }
            let _ = engine.metrics();
            ACTIVE.store(false, Ordering::SeqCst);
            assert_eq!(
                ALLOCS.load(Ordering::Relaxed),
                0,
                "threads={threads}, bytes={}",
                BYTES.load(Ordering::Relaxed)
            );
        });
    }
}

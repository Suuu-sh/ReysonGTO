//! Separate test binary avoids concurrent tests contaminating global allocation counts.
use cards::{Card, Combo};
use cfr::{Engine, EngineConfig, ScratchPrecision, StorageMode};
use ranges::Range;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tree::{GameTree, TreeConfig};
#[cfg(target_os = "macos")]
thread_local! { static OBSERVING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
struct Counter;
static ACTIVE: AtomicBool = AtomicBool::new(false);
static BYTES: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
// macOS std lazily allocates pthread parking primitives in Rayon's idle
// loop, not in CFR. A captured backtrace identified Mutex=64B and Condvar=48B.
// Observe their initialization BEFORE measurement, keyed by real worker IDs.
// A future std layout change must fail the bounded fixture, not silently skip it.
#[cfg(target_os = "macos")]
static INITIALIZING: AtomicBool = AtomicBool::new(false);
#[cfg(target_os = "macos")]
static PARKING: std::sync::Mutex<Vec<(std::thread::ThreadId, u8)>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(target_os = "macos")]
fn observe_parking(size: usize) {
    if !INITIALIZING.load(Ordering::Relaxed) || rayon::current_thread_index().is_none() {
        return;
    }
    let flag = match size {
        64 => 1,
        48 => 2,
        _ => return,
    };
    // Size alone is insufficient: broadcast/install also own lazy pthread
    // primitives. Only count parking primitives originating in Sleep::sleep.
    // Symbolization allocates, so suppress this observer recursively while
    // initialization runs (ACTIVE remains false throughout this phase).
    if OBSERVING.with(|guard| guard.replace(true)) {
        return;
    }
    let trace = std::backtrace::Backtrace::force_capture().to_string();
    let parking = trace.contains("rayon_core::sleep::")
        && (trace.contains("Sleep>::sleep") || trace.contains("Sleep::sleep"));
    OBSERVING.with(|guard| guard.set(false));
    if !parking {
        return;
    }
    let id = std::thread::current().id();
    let mut records = PARKING.lock().unwrap();
    if let Some((_, flags)) = records.iter_mut().find(|(worker, _)| *worker == id) {
        *flags |= flag;
    } else if records.len() < records.capacity() {
        // Never allocate inside the allocator observer. Capacity is initialized
        // on the external test thread before INITIALIZING becomes true.
        records.push((id, flag));
    }
}
#[cfg(target_os = "macos")]
fn begin_parking_initialization() {
    let mut records = PARKING.lock().unwrap(); // initialize observer's own mutex
    records.clear();
    records.reserve(16);
    INITIALIZING.store(true, Ordering::SeqCst);
}
#[cfg(target_os = "macos")]
fn await_parking_initialization(
    threads: usize,
    expected: Option<Vec<std::thread::ThreadId>>,
    previous: &[std::thread::ThreadId],
) -> Vec<std::thread::ThreadId> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let records = PARKING.lock().unwrap();
        let worker_ids: Vec<_> = expected.clone().unwrap_or_else(|| {
            records
                .iter()
                .filter(|(id, _)| !previous.contains(id))
                .map(|(id, _)| *id)
                .collect()
        });
        let ready = worker_ids.len() == threads
            && worker_ids.iter().all(|id| {
                records
                    .iter()
                    .any(|(worker, flags)| worker == id && *flags == 3)
            });
        if ready {
            INITIALIZING.store(false, Ordering::SeqCst);
            return worker_ids;
        }
        assert!(std::time::Instant::now() < deadline,
            "macOS Rayon parking initialization timed out: expected {worker_ids:?}, observed {records:?}; std pthread layout may have changed");
        drop(records);
        // Poll an explicit readiness condition, not an arbitrary warmup delay.
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(target_os = "macos")]
        observe_parking(layout.size());
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
    // Keep previous pools alive: asynchronous pool teardown must not contaminate
    // another pool's process-global allocation measurement.
    let mut live_engines = Vec::new();
    #[cfg(target_os = "macos")]
    let mut known_workers = Vec::new();
    for (threads, storage_mode, scratch_precision) in [
        (1, StorageMode::F32, ScratchPrecision::F64),
        (2, StorageMode::F32, ScratchPrecision::F64),
        (1, StorageMode::I16, ScratchPrecision::F64),
        (2, StorageMode::I16, ScratchPrecision::F64),
        (1, StorageMode::F32, ScratchPrecision::F32),
        (2, StorageMode::F32, ScratchPrecision::F32),
        (1, StorageMode::I16, ScratchPrecision::F32),
        (2, StorageMode::I16, ScratchPrecision::F32),
        (1, StorageMode::F32, ScratchPrecision::F32WideTotals),
        (2, StorageMode::F32, ScratchPrecision::F32WideTotals),
        (1, StorageMode::I16, ScratchPrecision::F32WideTotals),
        (2, StorageMode::I16, ScratchPrecision::F32WideTotals),
    ] {
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
        #[cfg(target_os = "macos")]
        begin_parking_initialization();
        let mut engine = Engine::new(
            tree,
            rr,
            EngineConfig {
                threads,
                storage_mode,
                scratch_precision,
                ..Default::default()
            },
        )
        .unwrap();
        #[cfg(target_os = "macos")]
        {
            // Let the newly built pool park BEFORE injecting any jobs. Wake
            // operations can initialize another worker's condvar on the waking
            // thread, so observing sizes after broadcast would misattribute it.
            let observed = await_parking_initialization(threads, None, &known_workers);
            if threads > 1 {
                let expected =
                    engine.with_pool(|_| rayon::broadcast(|_| std::thread::current().id()));
                assert!(
                    expected.len() == observed.len()
                        && expected.iter().all(|id| observed.contains(id)),
                    "parking fixture observed workers outside current engine pool"
                );
            }
            known_workers.extend(observed);
        }
        live_engines.push(engine);
        let engine = live_engines.last_mut().unwrap();
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
                "threads={threads}, storage={storage_mode:?}, scratch={scratch_precision:?}, bytes={}",
                BYTES.load(Ordering::Relaxed)
            );
        });
    }
}

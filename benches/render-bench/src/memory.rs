//! Peak-allocation tracking.
//!
//! Wall-clock timings alone would miss the point of this plan: the pipeline is
//! being changed as much to stop holding a 240 MB surface as to shave
//! milliseconds. This allocator records the high-water mark of live bytes so a
//! path can be charged for what it holds, not just what it does.
//!
//! Only the binaries in this crate install it, and it is a plain counter pair
//! on the hot path, so its overhead does not disturb the timings it sits
//! beside.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

pub struct TrackingAllocator;

impl TrackingAllocator {
    fn record_allocation(size: usize) {
        let live = LIVE.fetch_add(size, Ordering::Relaxed) + size;
        PEAK.fetch_max(live, Ordering::Relaxed);
    }

    fn record_release(size: usize) {
        LIVE.fetch_sub(size, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            Self::record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        Self::record_release(layout.size());
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            Self::record_allocation(layout.size());
        }
        pointer
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let moved = unsafe { System.realloc(pointer, layout, new_size) };
        if !moved.is_null() {
            Self::record_release(layout.size());
            Self::record_allocation(new_size);
        }
        moved
    }
}

/// Runs `body` with the peak counter rebased to the currently live bytes, and
/// returns the additional bytes it held at its worst moment.
pub fn peak_bytes_of<T>(body: impl FnOnce() -> T) -> (T, u64) {
    let baseline = LIVE.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);

    let value = body();

    let peak = PEAK.load(Ordering::Relaxed);
    (value, peak.saturating_sub(baseline) as u64)
}

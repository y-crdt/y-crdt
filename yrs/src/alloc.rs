use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

pub struct TracingAlloc {
    alloc: System,
    allocations: AtomicUsize,
    mem: AtomicUsize,
}

impl TracingAlloc {
    pub const fn new() -> Self {
        TracingAlloc {
            alloc: System,
            allocations: AtomicUsize::new(0),
            mem: AtomicUsize::new(0),
        }
    }

    pub fn reset(&mut self) {
        self.allocations.store(0, Ordering::SeqCst);
        self.mem.store(0, Ordering::SeqCst);
    }

    pub fn allocations(&mut self) -> usize {
        *self.allocations.get_mut()
    }

    pub fn mem_used(&mut self) -> usize {
        *self.mem.get_mut()
    }
}

unsafe impl GlobalAlloc for TracingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        self.allocations.fetch_add(1, Ordering::SeqCst);
        self.mem.fetch_add(layout.align(), Ordering::SeqCst);
        self.alloc.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        self.allocations.fetch_sub(1, Ordering::SeqCst);
        self.mem.fetch_sub(layout.align(), Ordering::SeqCst);
        self.alloc.dealloc(ptr, layout)
    }
}

unsafe impl Send for TracingAlloc {}
unsafe impl Sync for TracingAlloc {}

static GLOBAL: TracingAlloc = TracingAlloc::new();

//! One-shot stdin -> bounded JSON. All decoder/color allocations use this budget.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
struct Budget;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
fn reserve(bytes: usize) {
    let previous = LIVE.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
        n.checked_add(bytes)
            .filter(|v| *v <= soundshelf_core::image_analysis::WORKER_HEAP)
    });
    let old = match previous {
        Ok(n) => n,
        Err(_) => std::process::exit(78),
    };
    PEAK.fetch_max(old + bytes, Ordering::Relaxed);
}
// All codec/color backends are Rust and allocate through GlobalAlloc. Stack and
// mappings are excluded; deadline/kill supervision lives in the parent process.
unsafe impl GlobalAlloc for Budget {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        reserve(l.size());
        let p = unsafe { System.alloc(l) };
        if p.is_null() {
            std::process::exit(78);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        reserve(l.size());
        let p = unsafe { System.alloc_zeroed(l) };
        if p.is_null() {
            std::process::exit(78);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) };
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        // Count both allocations during a moving realloc, including its peak.
        reserve(n);
        let next = unsafe { Layout::from_size_align_unchecked(n, l.align()) };
        let out = unsafe { System.alloc(next) };
        if out.is_null() {
            std::process::exit(78);
        }
        unsafe {
            std::ptr::copy_nonoverlapping(p, out, l.size().min(n));
            System.dealloc(p, l);
        }
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
        out
    }
}
#[global_allocator]
static ALLOCATOR: Budget = Budget;
fn main() {
    soundshelf_core::image_analysis::worker_main(|| PEAK.load(Ordering::Relaxed));
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_allocator_budget_exits_only_its_child_process() {
        if std::env::var_os("CREATIVESHELF_ALLOCATOR_PROBE").is_some() {
            let bytes = Vec::<u8>::with_capacity(soundshelf_core::image_analysis::WORKER_HEAP + 1);
            std::hint::black_box(bytes);
            std::process::exit(99);
        }
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::actual_allocator_budget_exits_only_its_child_process",
            ])
            .env("CREATIVESHELF_ALLOCATOR_PROBE", "1")
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(78));
    }
}

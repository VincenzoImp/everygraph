// SPDX-License-Identifier: Apache-2.0 OR MIT

//! [`PAGES`]: buffers of [`LARGE_BLOCK`] bytes or more get pages of their own.
//!
//! A general allocator keeps blocks that were freed and hands them out again. glibc does
//! that for every block below its mmap threshold, which starts at 128 KiB and rises
//! after each free of a mapped block, up to 32 MiB (mallopt(3)). In the dictionary
//! builds of PR #158 and PR #203 the process held 1.2–2.3× the budget while the ledger
//! stayed within it (exploratory runs). A block mapped on its own is given back to the
//! system when it is freed, so a freed large buffer is never kept. Smaller blocks go to
//! the global allocator; glibc maps none of them, so the blocks that come through here
//! never raise its threshold. Blocks that other code takes from the global allocator
//! directly still can.
//!
//! Two costs come with it, which the planner's headroom carries:
//!
//! - A mapped block holds its size rounded up to whole pages, and a ledger counts the
//!   size: at most one page more per block, 3.1 % at the smallest mapped size.
//! - Every live mapped block is one kernel mapping, unless it touches a neighbour. A
//!   process may hold `vm.max_map_count` mappings (65,530 by default on Linux); past
//!   that, `mmap` fails and so does the allocation. The containers that use these pages
//!   allocate fallibly, so a build then ends with `AllocationFailed` instead of an
//!   abort. Reaching the limit takes about 65,000 live blocks, 8 GiB or more of them.
//!
//! Why safe code is not enough: std has no anonymous maps, and an allocator is an
//! `unsafe impl`. Each `unsafe` block is one call: `mmap`, `mremap` or `munmap` of a
//! region this module mapped, `sysconf`, a copy between two live blocks, or the global
//! allocator's `deallocate`, `grow` or `shrink` with the caller's pointer. Miri runs the tests below
//! (CONTRIBUTING.md, "`unsafe`").

#![allow(unsafe_code)]

use std::alloc::{AllocError, Allocator, Global, Layout};
use std::ptr::{self, NonNull};

use everygraph_core::Pages;

/// Blocks of this size or more are mapped on their own: 128 KiB, glibc's own mmap
/// threshold before it rises. At 1 MiB the blocks between the two stayed in glibc's
/// arenas, and the 500 M-key dictionary build held 1.08–1.10× its ledger against 0.95×
/// at 128 KiB (exploratory A/B, PR #219).
pub const LARGE_BLOCK: usize = 128 << 10;

/// The page backend of a native build: give it to
/// [`Budget::with_pages`](everygraph_core::Budget::with_pages).
///
/// On Unix, blocks of [`LARGE_BLOCK`] bytes or more are anonymous private maps, at a
/// multiple of 2 MiB from 2 MiB on; freeing one unmaps it. Smaller blocks, and blocks
/// aligned to more than a page, come from the global allocator. Elsewhere every block
/// does.
pub static PAGES: Pages = Pages::new("anonymous maps", &Maps);

struct Maps;

/// Whether a block of `layout` is mapped on its own.
fn mapped(layout: Layout) -> bool {
    cfg!(unix) && layout.size() >= LARGE_BLOCK && layout.align() <= sys::page()
}

/// Whether a mapped block is resized in place by the kernel (`mremap`, Linux only; not
/// under Miri, which does not have it).
const REMAP: bool = cfg!(any(target_os = "linux", target_os = "android")) && !cfg!(miri);

// SAFETY: a block is mapped exactly when `mapped(layout)`, a function of the layout
// alone, and `deallocate` gets the layout the block was allocated with (or one of the
// same size and alignment), so it frees each block the way it was made. A mapped block
// is `layout.size()` bytes of fresh pages, zeroed, page-aligned, owned by no one else
// until it is unmapped. `grow` and `shrink` pass a block that is not mapped before or
// after to `Global`, move a mapped block to a mapped size with `mremap`, which keeps
// its bytes and gives the same kind of block, and make any other change by allocate,
// copy, deallocate, which keeps the rule too.
unsafe impl Allocator for Maps {
    fn allocate(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if !mapped(layout) {
            return Global.allocate(layout);
        }
        let start = sys::map(layout.size()).ok_or(AllocError)?;
        Ok(NonNull::slice_from_raw_parts(start, layout.size()))
    }

    fn allocate_zeroed(&self, layout: Layout) -> Result<NonNull<[u8]>, AllocError> {
        if mapped(layout) {
            // Fresh anonymous pages are zero.
            self.allocate(layout)
        } else {
            Global.allocate_zeroed(layout)
        }
    }

    unsafe fn deallocate(&self, ptr: NonNull<u8>, layout: Layout) {
        if mapped(layout) {
            // SAFETY: the block is mapped, so `allocate` made it with `sys::map` of this
            // size; the caller guarantees it is live and not used again.
            unsafe { sys::unmap_block(ptr, layout.size()) };
        } else {
            // SAFETY: the block is not mapped, so `allocate` took it from `Global` with
            // this layout; the caller guarantees it is live and not used again.
            unsafe { Global.deallocate(ptr, layout) };
        }
    }

    unsafe fn grow(
        &self,
        ptr: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        if !mapped(old) && !mapped(new) {
            // SAFETY: the block came from `Global` with `old`; the caller's guarantees
            // for `grow` are passed on unchanged.
            return unsafe { Global.grow(ptr, old, new) };
        }
        if REMAP && mapped(old) && mapped(new) {
            // SAFETY: `ptr` is a mapped block of `old.size()` bytes (the caller's
            // guarantee and `mapped(old)`), and the caller gives it up on success.
            return unsafe { sys::remap(ptr, old.size(), new.size()) }
                .map(|start| NonNull::slice_from_raw_parts(start, new.size()))
                .ok_or(AllocError);
        }
        // SAFETY: the caller's guarantees for `grow`, passed on unchanged.
        unsafe { default_resize(self, ptr, old, new) }
    }

    unsafe fn shrink(
        &self,
        ptr: NonNull<u8>,
        old: Layout,
        new: Layout,
    ) -> Result<NonNull<[u8]>, AllocError> {
        if !mapped(old) && !mapped(new) {
            // SAFETY: as in `grow`, for `shrink`.
            return unsafe { Global.shrink(ptr, old, new) };
        }
        if REMAP && mapped(old) && mapped(new) {
            // SAFETY: as in `grow`.
            return unsafe { sys::remap(ptr, old.size(), new.size()) }
                .map(|start| NonNull::slice_from_raw_parts(start, new.size()))
                .ok_or(AllocError);
        }
        // SAFETY: the caller's guarantees for `shrink`, passed on unchanged.
        unsafe { default_resize(self, ptr, old, new) }
    }
}

/// Moves a block of `old` to a new block of `new`: allocate, copy the smaller of the
/// two sizes, deallocate. What the trait's `grow` and `shrink` do by default.
///
/// # Safety
///
/// `ptr` is a live block of `maps` with layout `old`; on success the caller uses only
/// the new block.
unsafe fn default_resize(
    maps: &Maps,
    ptr: NonNull<u8>,
    old: Layout,
    new: Layout,
) -> Result<NonNull<[u8]>, AllocError> {
    let block = maps.allocate(new)?;
    // SAFETY: both blocks are live, at least this long, and distinct (the new one was
    // just allocated).
    unsafe {
        ptr::copy_nonoverlapping(
            ptr.as_ptr(),
            block.cast::<u8>().as_ptr(),
            old.size().min(new.size()),
        );
    };
    // SAFETY: the caller's guarantee: `ptr` is live with layout `old`, and is not used
    // again.
    unsafe { maps.deallocate(ptr, old) };
    Ok(block)
}

#[cfg(unix)]
mod sys {
    use std::ptr::{self, NonNull};
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Maps of this size or more start at a multiple of it, so that the kernel can back
    /// them with transparent huge pages where it is allowed to.
    pub(super) const HUGE_PAGE: usize = 2 << 20;

    /// The size of a page, read once.
    pub(super) fn page() -> usize {
        static PAGE: AtomicUsize = AtomicUsize::new(0);
        let mut page = PAGE.load(Ordering::Relaxed);
        if page == 0 {
            // SAFETY: `sysconf` takes an integer and returns one; it touches no memory
            // of ours.
            let read = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
            page = usize::try_from(read)
                .ok()
                .filter(|p| p.is_power_of_two())
                .unwrap_or(4096);
            PAGE.store(page, Ordering::Relaxed);
        }
        page
    }

    /// Maps `len` bytes of fresh pages, read-write; at a multiple of [`HUGE_PAGE`]
    /// when `len` is at least that. The region is `len` rounded up to whole pages,
    /// which [`unmap_block`] frees.
    pub(super) fn map(len: usize) -> Option<NonNull<u8>> {
        let rounded = len.checked_next_multiple_of(page())?;
        // Miri unmaps only whole regions, so under Miri maps are page-aligned only.
        if rounded < HUGE_PAGE || cfg!(miri) {
            return raw_map(rounded);
        }
        let padded = rounded.checked_add(HUGE_PAGE)?;
        let base = raw_map(padded)?.as_ptr();
        let head = base.addr().wrapping_neg() % HUGE_PAGE;
        let start = base.wrapping_add(head);
        let tail = padded - head - rounded;
        // Head and tail are whole pages, because `base`, `HUGE_PAGE` and `rounded` are.
        if head > 0 {
            // SAFETY: `base .. start` is the start of the map made above, outside the
            // block that is returned.
            unsafe { unmap(base, head) };
        }
        if tail > 0 {
            // SAFETY: the `tail` bytes after the block are the end of the same map.
            unsafe { unmap(start.wrapping_add(rounded), tail) };
        }
        NonNull::new(start)
    }

    /// Frees a block of `len` bytes that [`map`] returned.
    ///
    /// # Safety
    ///
    /// `start` came from `map(len)`, and nothing uses the block any more.
    pub(super) unsafe fn unmap_block(start: NonNull<u8>, len: usize) {
        let rounded = len.next_multiple_of(page());
        // SAFETY: the caller's guarantee: `map(len)` mapped `rounded` bytes at `start`.
        unsafe { unmap(start.as_ptr(), rounded) };
    }

    fn raw_map(len: usize) -> Option<NonNull<u8>> {
        // SAFETY: an anonymous private map at an address the kernel chooses overlaps
        // nothing the process holds; no file descriptor is involved (`-1`, offset 0).
        let start = unsafe {
            libc::mmap(
                ptr::null_mut(),
                len,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_PRIVATE | libc::MAP_ANON,
                -1,
                0,
            )
        };
        if start == libc::MAP_FAILED {
            return None;
        }
        NonNull::new(start.cast())
    }

    /// Resizes a block of `old_len` bytes that [`map`] returned to `new_len` bytes,
    /// keeping its bytes; the kernel may move it. `None` if it could not, and then the
    /// old block is unchanged.
    ///
    /// # Safety
    ///
    /// `start` came from `map(old_len)`; on success the caller uses only the new block.
    pub(super) unsafe fn remap(
        start: NonNull<u8>,
        old_len: usize,
        new_len: usize,
    ) -> Option<NonNull<u8>> {
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let old = old_len.next_multiple_of(page());
            let new = new_len.checked_next_multiple_of(page())?;
            // SAFETY: the caller's guarantee: `start` is a map of `old` bytes; with
            // `MREMAP_MAYMOVE` the kernel picks the new place itself.
            let moved =
                unsafe { libc::mremap(start.as_ptr().cast(), old, new, libc::MREMAP_MAYMOVE) };
            if moved == libc::MAP_FAILED {
                return None;
            }
            NonNull::new(moved.cast())
        }
        #[cfg(not(any(target_os = "linux", target_os = "android")))]
        {
            let _ = (start, old_len, new_len);
            unreachable!("mremap is Linux only")
        }
    }

    /// Unmaps the pages of `start .. start + len`.
    ///
    /// # Safety
    ///
    /// This module mapped the region, `start` is page-aligned, and nothing refers to
    /// the region any more.
    unsafe fn unmap(start: *mut u8, len: usize) {
        // SAFETY: the caller's guarantee.
        let status = unsafe { libc::munmap(start.cast(), len) };
        // Only an invalid argument makes munmap fail, which would be a bug here.
        debug_assert_eq!(status, 0, "munmap of {len} bytes failed");
    }
}

#[cfg(not(unix))]
mod sys {
    use std::ptr::NonNull;

    pub(super) fn page() -> usize {
        unreachable!("only Unix maps blocks")
    }

    pub(super) fn map(_: usize) -> Option<NonNull<u8>> {
        unreachable!("only Unix maps blocks")
    }

    pub(super) unsafe fn unmap_block(_: NonNull<u8>, _: usize) {
        unreachable!("only Unix maps blocks")
    }

    pub(super) unsafe fn remap(_: NonNull<u8>, _: usize, _: usize) -> Option<NonNull<u8>> {
        unreachable!("only Unix maps blocks")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pages() -> &'static (dyn Allocator + Sync) {
        PAGES.allocator()
    }

    /// The bytes a test checks in a block of `size`: the ends and a few between, so
    /// that the test runs in seconds under Miri, which interprets every byte it reads.
    fn probes(size: usize) -> impl Iterator<Item = usize> {
        (0..16).map(move |i| i * (size - 1) / 15)
    }

    /// Blocks on both sides of the threshold start zeroed, hold what is written to
    /// them, and are freed without error.
    #[test]
    fn blocks_below_and_above_the_threshold_work() {
        for size in [
            1,
            4096,
            LARGE_BLOCK - 1,
            LARGE_BLOCK,
            LARGE_BLOCK + 1,
            3 << 20,
        ] {
            let layout = Layout::from_size_align(size, 8).unwrap();
            let block = pages().allocate_zeroed(layout).unwrap();
            let start = block.cast::<u8>().as_ptr();
            // SAFETY: `at < size`: every byte is inside the block, which nothing else uses.
            let byte = |at: usize| unsafe { start.add(at) };
            for at in probes(size) {
                // SAFETY: as above.
                assert_eq!(unsafe { byte(at).read() }, 0);
            }
            for at in probes(size) {
                // SAFETY: as above.
                unsafe { byte(at).write(7) };
            }
            for at in probes(size) {
                // SAFETY: as above.
                assert_eq!(unsafe { byte(at).read() }, 7);
            }
            // SAFETY: allocated above with `layout`, and not used again.
            unsafe { pages().deallocate(block.cast(), layout) };
        }
    }

    /// A vector that grows across the threshold, grows again, and shrinks back below
    /// it keeps its bytes: each step frees the old block the way it was made.
    #[test]
    fn growth_and_shrinking_cross_the_threshold() {
        let mut v: Vec<u8, _> = Vec::new_in(pages());
        v.extend_from_slice(b"pages");
        for capacity in [LARGE_BLOCK, 3 << 20, 9 << 20] {
            v.try_reserve_exact(capacity - v.len()).unwrap();
            assert_eq!(v.capacity(), capacity);
            assert_eq!(&v[..5], b"pages");
        }
        v.shrink_to(2 << 20);
        assert_eq!(&v[..5], b"pages");
        v.shrink_to_fit();
        assert_eq!((v.capacity(), &v[..]), (5, &b"pages"[..]));
    }

    /// Maps of 2 MiB or more start on a 2 MiB boundary, where huge pages can back them;
    /// a block aligned to more than a page is not mapped and still aligned.
    #[test]
    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "Miri's maps are page-aligned only")]
    fn large_maps_are_aligned_for_huge_pages() {
        use sys::HUGE_PAGE;
        for size in [HUGE_PAGE, 5 << 20] {
            let mut v: Vec<u8, _> = Vec::new_in(pages());
            v.try_reserve_exact(size).unwrap();
            assert_eq!(v.as_ptr().addr() % HUGE_PAGE, 0);
        }
        let wide = Layout::from_size_align(LARGE_BLOCK, 1 << 16).unwrap();
        assert!(!mapped(wide));
        let block = pages().allocate(wide).unwrap();
        assert_eq!(block.cast::<u8>().as_ptr().addr() % (1 << 16), 0);
        // SAFETY: allocated above with `wide`, and not used again.
        unsafe { pages().deallocate(block.cast(), wide) };
    }

    /// The core's containers allocate from the accountant's pages: their large buffers
    /// start on a 2 MiB boundary, which glibc's `malloc` never gives (its blocks start
    /// 16 bytes into a page).
    #[test]
    #[cfg(unix)]
    #[cfg_attr(miri, ignore = "Miri's maps are page-aligned only")]
    fn the_core_containers_allocate_from_the_budget_s_pages() {
        use everygraph_core::{Budget, Memory, ScratchPool, SegVec};
        use std::sync::Arc;
        use sys::HUGE_PAGE;

        let memory = Memory::from(Arc::new(Budget::unlimited().with_pages(PAGES)));
        assert_eq!(memory.pages().name(), PAGES.name());
        let ranks = SegVec::try_with_len(&memory, "ranks", 1 << 20, 0_u32).unwrap();
        assert_eq!(ranks.as_slice().unwrap().as_ptr().addr() % HUGE_PAGE, 0);
        let pool = ScratchPool::try_new(&memory, "tables", 2, 1 << 20, 0_u32).unwrap();
        let lent = pool.with(|buffer| buffer.as_ptr().addr()).unwrap();
        assert_eq!(lent % HUGE_PAGE, 0);
    }
}

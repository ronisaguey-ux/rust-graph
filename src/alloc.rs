//! A size-class allocator. A parse makes millions of short-lived small strings,
//! and musl's malloc is slow at exactly that; recycling blocks from a free list
//! per power-of-two class took the parse from allocator-bound to lexer-bound.
//! Anything over `LARGEST` goes to the system allocator, so a big buffer is
//! still returned on free. Small blocks are recycled, never returned, which
//! bounds them by the peak of live small data.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::UnsafeCell;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

const SMALLEST_SHIFT: usize = 4;
const CLASSES: usize = 9;
const LARGEST: usize = 1 << (SMALLEST_SHIFT + CLASSES - 1);
const CHUNK: usize = 256 * 1024;

struct State {
    free: [*mut u8; CLASSES],
    bump: *mut u8,
    left: usize,
}

pub struct Pool {
    locked: AtomicBool,
    state: UnsafeCell<State>,
}

// The lock below serialises every access to `state`.
unsafe impl Sync for Pool {}

impl Pool {
    pub const fn new() -> Self {
        Pool {
            locked: AtomicBool::new(false),
            state: UnsafeCell::new(State { free: [null_mut(); CLASSES], bump: null_mut(), left: 0 }),
        }
    }

    fn with<T>(&self, work: impl FnOnce(&mut State) -> T) -> T {
        while self.locked.compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed).is_err() {
            std::hint::spin_loop();
        }
        // Safety: the flag is held, so this is the only reference.
        let result = work(unsafe { &mut *self.state.get() });
        self.locked.store(false, Ordering::Release);
        result
    }
}

/// The class for a layout, or None when the system allocator should serve it.
fn class_of(layout: &Layout) -> Option<usize> {
    let size = layout.size().max(layout.align());
    if size > LARGEST {
        return None;
    }
    let rounded = size.max(1 << SMALLEST_SHIFT).next_power_of_two();
    Some(rounded.trailing_zeros() as usize - SMALLEST_SHIFT)
}

unsafe impl GlobalAlloc for Pool {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let Some(class) = class_of(&layout) else { return System.alloc(layout) };
        let size = 1usize << (class + SMALLEST_SHIFT);
        self.with(|state| {
            let head = state.free[class];
            if !head.is_null() {
                state.free[class] = *(head as *mut *mut u8);
                return head;
            }
            // Classes are powers of two and chunks are aligned to the largest,
            // so padding to the class aligns the block for any layout it serves.
            let padding = (size - state.bump as usize % size) % size;
            if state.left < size + padding {
                let chunk = System.alloc(Layout::from_size_align_unchecked(CHUNK, LARGEST));
                if chunk.is_null() {
                    return null_mut();
                }
                state.bump = chunk;
                state.left = CHUNK;
                let block = state.bump;
                state.bump = block.add(size);
                state.left -= size;
                return block;
            }
            let block = state.bump.add(padding);
            state.bump = block.add(size);
            state.left -= size + padding;
            block
        })
    }

    unsafe fn dealloc(&self, block: *mut u8, layout: Layout) {
        let Some(class) = class_of(&layout) else { return System.dealloc(block, layout) };
        self.with(|state| {
            *(block as *mut *mut u8) = state.free[class];
            state.free[class] = block;
        });
    }

    unsafe fn realloc(&self, block: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
        let (old, new) = (class_of(&layout), class_of(&new_layout));
        if old.is_none() && new.is_none() {
            return System.realloc(block, layout, new_size);
        }
        if old == new {
            return block;
        }
        let moved = self.alloc(new_layout);
        if !moved.is_null() {
            std::ptr::copy_nonoverlapping(block, moved, layout.size().min(new_size));
            self.dealloc(block, layout);
        }
        moved
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_are_aligned_recycled_and_keep_their_bytes_when_grown() {
        let pool = Pool::new();
        unsafe {
            let small = pool.alloc(Layout::from_size_align(8, 8).unwrap());
            let layout = Layout::from_size_align(24, 32).unwrap();
            let first = pool.alloc(layout);
            assert_eq!(first as usize % 32, 0, "a block after a smaller class is still aligned");
            assert_ne!(small, first);
            first.write_bytes(7, 24);
            pool.dealloc(first, layout);
            let second = pool.alloc(layout);
            assert_eq!(first, second);
            second.write_bytes(9, 24);
            let grown = pool.realloc(second, layout, 100);
            assert_ne!(grown, second);
            assert_eq!(*grown.add(23), 9);
            pool.dealloc(grown, Layout::from_size_align(100, 32).unwrap());
        }
    }

    #[test]
    fn a_large_request_is_the_system_allocators() {
        assert_eq!(class_of(&Layout::from_size_align(LARGEST, 8).unwrap()), Some(CLASSES - 1));
        assert_eq!(class_of(&Layout::from_size_align(LARGEST + 1, 8).unwrap()), None);
        assert_eq!(class_of(&Layout::from_size_align(1, 1).unwrap()), Some(0));
    }
}

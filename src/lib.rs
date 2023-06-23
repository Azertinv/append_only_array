#![feature(maybe_uninit_uninit_array)]
//! A Thread safe append only array with a fixed size. Allows reader's to read
//! from the array with no atomic operations.

use core::mem::MaybeUninit;
use core::cell::UnsafeCell;
use core::result::Result;
use core::default::Default;
use core::sync::atomic::Ordering;
use std::ops::Deref;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, PartialEq)]
pub enum AppendArrayError {
    ArrayFull,
}

pub struct AppendArray<T, const N: usize> {
    ticket: AtomicUsize,
    len: AtomicUsize,
    array: [MaybeUninit<UnsafeCell<T>>; N],
}

unsafe impl<T: Send, const N: usize> Send for AppendArray<T, N> {}
unsafe impl<T: Send, const N: usize> Sync for AppendArray<T, N> {}

impl<T, const N: usize> Deref for AppendArray<T, N> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        unsafe {
            core::slice::from_raw_parts(
                self.array.as_ptr() as *const T,
                self.len.load(Ordering::Relaxed))
        }
    }
}

impl<T, const N: usize> Default for AppendArray<T, N> {
    fn default() -> Self {
        AppendArray {
            ticket: AtomicUsize::new(0),
            len: AtomicUsize::new(0),
            array: MaybeUninit::uninit_array(),
        }
    }
}

impl<T, const N: usize> AppendArray<T, N> {
    /// Append an element to the back of the array, returns the index of the 
    /// element
    pub fn append(&self, item: T) -> Result<usize, AppendArrayError> {
        let ticket = self.ticket.fetch_add(1, Ordering::Relaxed);
        if ticket >= N {
            self.ticket.fetch_sub(1, Ordering::Relaxed);
            return Err(AppendArrayError::ArrayFull);
        }
        unsafe {
            UnsafeCell::raw_get(self.array[ticket].as_ptr()).write(item);
        }
        Ok(self.len.fetch_add(1, Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_works() {
        let array = AppendArray::<u32, 1024>::default();
        let idx = array.append(31).unwrap();
        assert_eq!(array[idx], 31);
        assert_eq!(idx, 0);
    }

    #[test]
    fn stress() {
        const ITERS: usize = 1024;
        const THREADS: usize = 8;
        const TOTAL: usize = ITERS * THREADS;
        let array = AppendArray::<u8, TOTAL>::default();
        std::thread::scope(|s| {
            let array = &array;
            for i in 0..THREADS {
                s.spawn(move || {
                    for j in 0..ITERS {
                        array.append((i * j) as u8).unwrap();
                    }
                });
            }
        });
        assert_eq!(array.len(), TOTAL);
        assert_eq!(array.append(0), Err(AppendArrayError::ArrayFull));
    }
}

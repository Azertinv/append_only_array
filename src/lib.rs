#![no_std]
#![feature(maybe_uninit_uninit_array)]
//! A Thread safe append only array with a fixed size. Allows reader's to read
//! from the array with no atomic operations.

use core::cell::UnsafeCell;
use core::default::Default;
use core::fmt::Debug;
use core::mem::MaybeUninit;
use core::ops::{Deref, Drop};
use core::result::Result;
use core::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, PartialEq)]
pub enum AppendArrayError {
    ArrayFull,
}

#[derive(Debug)]
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
                self.len.load(Ordering::Relaxed),
            )
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

impl<T, const N: usize> Drop for AppendArray<T, N> {
    fn drop(&mut self) {
        for i in 0..self.len.load(Ordering::Relaxed) {
            unsafe {
                self.array[i].assume_init_drop();
            }
        }
    }
}

impl<T, const N: usize> AppendArray<T, N> {
    /// Append an element to the end of the array, returns the index of the
    /// element or an error if the array is full.
    pub fn append(&self, item: T) -> Result<usize, AppendArrayError> {
        // Get the current ticket and increase it
        let ticket = self.ticket.fetch_add(1, Ordering::Relaxed);

        // If the ticket is too big it means the array is full
        if ticket >= N {
            self.ticket.fetch_sub(1, Ordering::Relaxed);
            return Err(AppendArrayError::ArrayFull);
        }

        // Store the item in the array
        unsafe {
            UnsafeCell::raw_get(self.array[ticket].as_ptr()).write(item);
        }

        // Another thread may have been able to write the next item in the array
        // before this one and try to increase the length, which could make the
        // the array use an uninitialized value in the array.
        // Therefore we need to wait for our turn.
        while self.len.load(Ordering::Relaxed) != ticket {
            core::hint::spin_loop();
        }

        // The item is in the array and it's now our turn to increase the length
        self.len.fetch_add(1, Ordering::Relaxed);

        // Return the index of the item we just inserted
        Ok(ticket)
    }
}

#[cfg(test)]
#[macro_use]
extern crate std;

#[cfg(test)]
mod tests {

    use super::*;
    use std::boxed::Box;

    #[test]
    fn it_works() {
        let array = AppendArray::<u32, 1024>::default();
        let idx_0 = array.append(31).unwrap();
        let idx_1 = array.append(35).unwrap();
        assert_eq!(array[idx_0], 31);
        assert_eq!(idx_0, 0);
        assert_eq!(array[idx_1], 35);
        assert_eq!(idx_1, 1);
        assert_eq!(array.len(), 2);
    }

    #[test]
    fn stress() {
        const ITERS: usize = 0x1_000;
        const THREADS: usize = 8;
        const TOTAL: usize = ITERS * THREADS;
        // put the array in a box to not blow up our stack
        let array = Box::new(AppendArray::<usize, TOTAL>::default());
        std::thread::scope(|s| {
            let array = &array;
            for i in 0..THREADS {
                s.spawn(move || {
                    for j in 0..ITERS {
                        array.append(i * ITERS + j).unwrap();
                    }
                });
            }
        });
        assert_eq!(array.len(), TOTAL);
        assert_eq!(array.append(0), Err(AppendArrayError::ArrayFull));
        for i in 0..TOTAL {
            assert!(array.contains(&i));
        }
    }

    #[test]
    #[should_panic]
    fn oob() {
        let array = AppendArray::<&u32, 1024>::default();
        println!("{:?}", &array[0]);
    }

    struct ToDrop<'a>(&'a AtomicUsize);
    impl Drop for ToDrop<'_> {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn dropping() -> Result<(), AppendArrayError> {
        let count = AtomicUsize::new(0);
        {
            let array = AppendArray::<ToDrop, 3>::default();
            array.append(ToDrop(&count))?;
            array.append(ToDrop(&count))?;
            array.append(ToDrop(&count))?;
        }
        assert_eq!(count.load(Ordering::Relaxed), 3);
        Ok(())
    }
}

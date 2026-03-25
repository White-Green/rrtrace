use crossbeam_utils::CachePadded;
use std::sync::atomic::AtomicPtr;
use std::sync::{Arc, atomic};
use std::{iter, ptr};

pub struct ObjectScatter<T> {
    array: Arc<[CachePadded<[AtomicPtr<T>; 16]>]>,
    index: usize,
}

pub struct ObjectScatterReceiver<T> {
    array: Arc<[CachePadded<[AtomicPtr<T>; 16]>]>,
    slot: usize,
    index: usize,
}

impl<T: Send> ObjectScatter<T> {
    pub fn new(
        count: usize,
    ) -> (
        ObjectScatter<T>,
        impl ExactSizeIterator<Item = ObjectScatterReceiver<T>>,
    ) {
        assert!(count > 0);
        let array = Arc::from(Vec::from_iter(
            iter::from_fn(|| Some([const { AtomicPtr::new(ptr::null_mut()) }; 16]))
                .map(CachePadded::new)
                .take(count),
        ));
        let scatter = ObjectScatter {
            array: Arc::clone(&array),
            index: 0,
        };
        let receiver_iter = (0..count).map(move |slot| ObjectScatterReceiver {
            array: Arc::clone(&array),
            slot,
            index: 0,
        });
        (scatter, receiver_iter)
    }

    pub fn send(&mut self, value: T) {
        let ptr = Box::new(value);
        let ptr = Box::into_raw(ptr);
        let i = self.index;
        let slot = i % self.array.len();
        let index = i / self.array.len();
        let old_ptr = self.array[slot][index].swap(ptr, atomic::Ordering::AcqRel);
        if !old_ptr.is_null() {
            unsafe {
                drop(Box::from_raw(old_ptr));
            }
        }
        self.index = self.index + 1;
        if self.index >= self.array.len() * 16 {
            self.index = 0;
        }
    }
}

impl<T> Drop for ObjectScatter<T> {
    fn drop(&mut self) {
        self.array
            .iter()
            .flat_map(|array| array.iter())
            .for_each(|ptr| {
                let ptr = ptr.swap(ptr::null_mut(), atomic::Ordering::Acquire);
                if !ptr.is_null() {
                    unsafe {
                        drop(Box::from_raw(ptr));
                    }
                }
            });
    }
}

impl<T: Send> ObjectScatterReceiver<T> {
    pub fn receive(&mut self) -> Option<Box<T>> {
        let slot = self.slot;
        let index = self.index;
        let ptr = self.array[slot][index].swap(ptr::null_mut(), atomic::Ordering::Acquire);
        self.index = self.index + 1;
        if self.index >= 16 {
            self.index = 0;
        }
        if ptr.is_null() {
            return None;
        }
        unsafe { Some(Box::from_raw(ptr)) }
    }
}

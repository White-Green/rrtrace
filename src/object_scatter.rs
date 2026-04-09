use std::sync::atomic::AtomicPtr;
use std::sync::{Arc, atomic};
use std::{iter, marker::PhantomData, ptr, rc::Rc};

#[repr(align(128))]
struct Line<T> {
    array: [AtomicPtr<T>; 16],
}

impl<T> Line<T> {
    fn new() -> Line<T> {
        Line {
            array: [const { AtomicPtr::new(ptr::null_mut()) }; 16],
        }
    }

    #[inline(always)]
    fn get(&self, index: usize) -> &AtomicPtr<T> {
        &self.array[index]
    }
}

impl<T> Drop for Line<T> {
    fn drop(&mut self) {
        self.array.iter_mut().for_each(|ptr| {
            let ptr = ptr.get_mut();
            if !ptr.is_null() {
                unsafe {
                    drop(Box::from_raw(ptr));
                }
            }
        })
    }
}

pub struct ObjectScatter<T> {
    array: Arc<[Line<T>]>,
    index: usize,
    _not_send: PhantomData<Rc<()>>,
}

pub struct ObjectScatterReceiver<T> {
    array: Arc<[Line<T>]>,
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
            iter::from_fn(|| Some(Line::new())).take(count),
        ));
        let scatter = ObjectScatter {
            array: Arc::clone(&array),
            index: 0,
            _not_send: PhantomData,
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
        let line = &self.array[slot];
        let old_ptr = line.get(index).swap(ptr, atomic::Ordering::Release);
        if !old_ptr.is_null() {
            unsafe {
                drop(Box::from_raw(old_ptr));
            }
        }
        self.index += 1;
        if self.index >= self.array.len() * 16 {
            self.index = 0;
        }
    }
}

impl<T: Send> ObjectScatterReceiver<T> {
    pub fn try_receive(&mut self) -> Option<Box<T>> {
        let line = &self.array[self.slot];
        for _ in 0..16 {
            let index = self.index;
            let ptr = line
                .get(index)
                .swap(ptr::null_mut(), atomic::Ordering::Acquire);
            self.index += 1;
            if self.index >= 16 {
                self.index = 0;
            }
            if !ptr.is_null() {
                return unsafe { Some(Box::from_raw(ptr)) };
            }
        }
        None
    }
}

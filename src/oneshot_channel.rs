use std::cell::UnsafeCell;
use std::mem::MaybeUninit;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, atomic};

struct OneshotSlot<T> {
    value: UnsafeCell<MaybeUninit<T>>,
    initialized: AtomicBool,
}

impl<T> Drop for OneshotSlot<T> {
    fn drop(&mut self) {
        if self.initialized.load(atomic::Ordering::Acquire) {
            unsafe {
                self.value.get_mut().assume_init_drop();
            }
        }
    }
}

pub struct OneshotSender<T> {
    slot: Arc<OneshotSlot<T>>,
}

unsafe impl<T: Send> Send for OneshotSender<T> {}

impl<T> OneshotSender<T> {
    #[inline(always)]
    pub fn send(self, value: T) {
        unsafe {
            MaybeUninit::write(&mut *self.slot.value.get(), value);
        }
        self.slot.initialized.store(true, atomic::Ordering::Release);
    }
}

pub struct OneshotReceiver<T> {
    slot: Arc<OneshotSlot<T>>,
}

unsafe impl<T: Send> Send for OneshotReceiver<T> {}

impl<T> OneshotReceiver<T> {
    #[inline(always)]
    pub fn try_receive(self) -> Result<T, OneshotReceiver<T>> {
        if self
            .slot
            .initialized
            .compare_exchange(
                true,
                false,
                atomic::Ordering::AcqRel,
                atomic::Ordering::Relaxed,
            )
            .is_ok()
        {
            let value = unsafe { (*self.slot.value.get()).assume_init_read() };
            Ok(value)
        } else {
            Err(self)
        }
    }
}

pub fn channel<T>() -> (OneshotSender<T>, OneshotReceiver<T>) {
    let slot = Arc::new(OneshotSlot {
        value: UnsafeCell::new(MaybeUninit::uninit()),
        initialized: AtomicBool::new(false),
    });
    let sender = OneshotSender {
        slot: Arc::clone(&slot),
    };
    let receiver = OneshotReceiver { slot };
    (sender, receiver)
}

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, atomic};

#[derive(Debug, Clone)]
pub struct UniversalNotifier {
    target: Arc<AtomicU32>,
}

impl UniversalNotifier {
    pub fn new() -> UniversalNotifier {
        UniversalNotifier {
            target: Arc::new(AtomicU32::new(0)),
        }
    }

    #[inline(always)]
    pub fn value(&self) -> u32 {
        self.target.load(atomic::Ordering::Relaxed)
    }

    #[inline(always)]
    pub fn wait(&self, v: u32) {
        atomic_wait::wait(&self.target, v)
    }

    #[inline(always)]
    pub fn notify(&self) {
        self.target.fetch_add(1, atomic::Ordering::Relaxed);
        atomic_wait::wake_all(&*self.target);
    }
}

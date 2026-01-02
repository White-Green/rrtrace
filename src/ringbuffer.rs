use std::sync::atomic::{self, AtomicU64};

#[repr(C)]
#[derive(Copy, Clone, Default)]
pub struct RRProfTraceEvent {
    pub timestamp_and_event_type: u64,
    pub data: u64,
}

pub const SIZE: usize = 65_536;
pub const MASK: usize = SIZE - 1;

#[repr(C, align(64))]
struct RRProfEventRingBufferWriter {
    write_index: AtomicU64,
    read_index_cache: u64,
}

#[repr(C, align(64))]
struct RRProfEventRingBufferReader {
    read_index: AtomicU64,
    write_index_cache: u64,
}

#[repr(C)]
pub struct RRProfEventRingBuffer {
    buffer: [RRProfTraceEvent; SIZE],
    writer: RRProfEventRingBufferWriter,
    reader: RRProfEventRingBufferReader,
}

impl RRProfEventRingBuffer {
    unsafe fn read(this: *mut Self, buffer: &mut [RRProfTraceEvent]) -> usize {
        unsafe {
            let read_index = (*this).reader.read_index.load(atomic::Ordering::Acquire);
            let write_index = (*this).reader.write_index_cache;
            let available = (write_index - read_index) as usize;
            let available = if available == 0 {
                (*this).reader.write_index_cache =
                    (*this).writer.write_index.load(atomic::Ordering::Acquire);
                let write_index = (*this).reader.write_index_cache;
                (write_index - read_index) as usize
            } else {
                available
            };
            let read_len = available.min(buffer.len());
            let buffer = &mut buffer[..read_len];

            let first_part_len = (&(*this).buffer)[read_index as usize & MASK..].len();
            if read_len <= first_part_len {
                buffer
                    .copy_from_slice(&(&(*this).buffer)[read_index as usize & MASK..][..read_len]);
            } else {
                buffer[..first_part_len]
                    .copy_from_slice(&(&(*this).buffer)[read_index as usize & MASK..]);
                buffer[first_part_len..]
                    .copy_from_slice(&(&(*this).buffer)[0..read_len - first_part_len]);
            }

            (*this)
                .reader
                .read_index
                .store(read_index + read_len as u64, atomic::Ordering::Release);
            read_len
        }
    }
}

pub struct EventRingBuffer {
    ringbuffer: *mut RRProfEventRingBuffer,
    drop: Option<Box<dyn FnOnce()>>,
}

impl EventRingBuffer {
    pub unsafe fn new(
        ringbuffer: *mut RRProfEventRingBuffer,
        drop: impl FnOnce() + 'static,
    ) -> Self {
        EventRingBuffer {
            ringbuffer,
            drop: Some(Box::new(drop)),
        }
    }

    pub fn read(&mut self, buffer: &mut [RRProfTraceEvent]) -> usize {
        unsafe { RRProfEventRingBuffer::read(self.ringbuffer, buffer) }
    }
}

impl Drop for EventRingBuffer {
    fn drop(&mut self) {
        self.drop.take().unwrap()();
    }
}

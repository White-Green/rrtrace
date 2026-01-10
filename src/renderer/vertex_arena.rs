use bytemuck::NoUninit;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::ops::Range;
use std::sync::atomic;
use wgpu::{Buffer, BufferAddress, BufferDescriptor, BufferUsages, Device, Queue};

#[derive(Debug, Clone, Copy, Ord, PartialOrd, Eq, PartialEq, Hash)]
pub struct AllocationId(usize);

impl AllocationId {
    fn new() -> AllocationId {
        static COUNTER: atomic::AtomicUsize = atomic::AtomicUsize::new(0);
        AllocationId(COUNTER.fetch_add(1, atomic::Ordering::Relaxed))
    }
}

pub struct VertexArena<T> {
    device: Device,
    queue: Queue,
    data: Vec<T>,
    gpu_buffer: Vec<Buffer>,
    max_buffer_size: u64,
    usage: BufferUsages,
    allocations: HashMap<AllocationId, Range<usize>>,
    free_list: FreeList,
    dirty_range: Range<usize>,
}

struct FreeList {
    by_start: BTreeMap<usize, usize>,
    by_size: BTreeSet<(usize, usize)>,
}

impl FreeList {
    fn new() -> Self {
        Self {
            by_start: BTreeMap::new(),
            by_size: BTreeSet::new(),
        }
    }

    fn alloc(&mut self, len: usize) -> Option<Range<usize>> {
        let &(size, start) = self.by_size.range((len, 0)..).next()?;
        self.by_size.remove(&(size, start));
        self.by_start.remove(&start);
        if size > len {
            let new_start = start + len;
            let new_size = size - len;
            self.by_start.insert(new_start, new_start + new_size);
            self.by_size.insert((new_size, new_start));
        }
        Some(start..start + len)
    }

    fn dealloc(&mut self, range: Range<usize>) {
        let mut start = range.start;
        let mut end = range.end;

        if let Some((&next_start, &next_end)) = self.by_start.range(end..).next() {
            if next_start == end {
                self.by_size.remove(&(next_end - next_start, next_start));
                self.by_start.remove(&next_start);
                end = next_end;
            }
        }

        if let Some((&prev_start, &prev_end)) = self.by_start.range(..start).next_back() {
            if prev_end == start {
                self.by_size.remove(&(prev_end - prev_start, prev_start));
                self.by_start.remove(&prev_start);
                start = prev_start;
            }
        }

        self.by_start.insert(start, end);
        self.by_size.insert((end - start, start));
    }
}

impl<T> Debug for VertexArena<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.data).finish()
    }
}

impl<T> VertexArena<T> {
    pub fn new(device: Device, queue: Queue, usage: BufferUsages) -> VertexArena<T> {
        let max_buffer_size = device.limits().max_buffer_size;
        let gpu_buffer = device.create_buffer(&BufferDescriptor {
            label: None,
            size: (max_buffer_size / size_of::<T>() as u64).min(256) * size_of::<T>() as u64,
            usage,
            mapped_at_creation: false,
        });
        VertexArena {
            data: Vec::new(),
            device,
            queue,
            gpu_buffer: vec![gpu_buffer],
            max_buffer_size,
            usage,
            allocations: HashMap::new(),
            free_list: FreeList::new(),
            dirty_range: usize::MAX..0,
        }
    }

    pub fn alloc(&mut self, len: usize) -> (AllocationId, &mut [T])
    where
        T: Default,
    {
        let id = AllocationId::new();
        let range = if let Some(range) = self.free_list.alloc(len) {
            range
        } else {
            let start = self.data.len();
            self.data.resize_with(start + len, T::default);
            start..start + len
        };

        self.allocations.insert(id, range.clone());
        self.dirty_range.start = self.dirty_range.start.min(range.start);
        self.dirty_range.end = self.dirty_range.end.max(range.end);

        let result = &mut self.data[range.clone()];
        assert_eq!(result.len(), len);
        (id, result)
    }

    pub fn dealloc(&mut self, id: AllocationId) {
        if let Some(range) = self.allocations.remove(&id) {
            self.free_list.dealloc(range);
        }
    }

    pub fn sync(&mut self)
    where
        T: NoUninit,
    {
        if self.dirty_range.start >= self.dirty_range.end {
            return;
        }

        let filled_buffer_len = self.max_buffer_size / size_of::<T>() as u64;
        let single_buffer_size_max = filled_buffer_len * size_of::<T>() as u64;
        if let [gpu_buffer] = self.gpu_buffer.as_slice() {
            let required_size = self.data.len() as u64 * size_of::<T>() as u64;
            if required_size > gpu_buffer.size() {
                if required_size <= self.max_buffer_size {
                    self.gpu_buffer = vec![self.device.create_buffer(&BufferDescriptor {
                        label: None,
                        size: required_size.next_power_of_two(),
                        usage: gpu_buffer.usage(),
                        mapped_at_creation: false,
                    })];
                } else {
                    let required_buffer_count = required_size.div_ceil(single_buffer_size_max);
                    let buffer_usages = gpu_buffer.usage();
                    if gpu_buffer.size() < single_buffer_size_max {
                        self.gpu_buffer.clear();
                    }
                    for _ in 1..required_buffer_count {
                        self.gpu_buffer
                            .push(self.device.create_buffer(&BufferDescriptor {
                                label: None,
                                size: single_buffer_size_max,
                                usage: buffer_usages,
                                mapped_at_creation: false,
                            }));
                    }
                }
                self.dirty_range = 0..self.data.len();
            }
        } else {
            let new_buffer_len = self.data.len().div_ceil(filled_buffer_len as usize);
            let usage = self.gpu_buffer[0].usage();
            for _ in self.gpu_buffer.len()..new_buffer_len {
                self.gpu_buffer
                    .push(self.device.create_buffer(&BufferDescriptor {
                        label: None,
                        size: single_buffer_size_max,
                        usage,
                        mapped_at_creation: false,
                    }));
            }
        }

        if let [gpu_buffer] = self.gpu_buffer.as_slice() {
            let dirty_data = &self.data[self.dirty_range.clone()];
            let offset = (self.dirty_range.start * size_of::<T>()) as BufferAddress;
            let bytes: &[u8] = bytemuck::cast_slice(dirty_data);
            self.queue.write_buffer(gpu_buffer, offset, bytes);
        } else {
            let start_block = self.dirty_range.start / filled_buffer_len as usize;
            let start_item = self.dirty_range.start % filled_buffer_len as usize;
            let end_block = self.dirty_range.end / filled_buffer_len as usize;
            let end_item = self.dirty_range.end % filled_buffer_len as usize;
            match &self.gpu_buffer[start_block..=end_block] {
                [] => unreachable!(),
                [buffer] => {
                    let dirty_data = &self.data[self.dirty_range.clone()];
                    let offset = (start_item * size_of::<T>()) as BufferAddress;
                    let bytes: &[u8] = bytemuck::cast_slice(dirty_data);
                    self.queue.write_buffer(buffer, offset, bytes);
                }
                [first, mid @ .., last] => {
                    let data = &self.data[start_block * filled_buffer_len as usize
                        ..((end_block + 1) * filled_buffer_len as usize).min(self.data.len())];
                    let mut data_iter = data.chunks(filled_buffer_len as usize);
                    let first_chunk = data_iter.next().unwrap();
                    let last_chunk = data_iter.next_back().unwrap();
                    self.queue.write_buffer(
                        first,
                        (start_item * size_of::<T>()) as BufferAddress,
                        bytemuck::cast_slice(&first_chunk[start_item..]),
                    );
                    if end_item > 0 {
                        self.queue.write_buffer(
                            last,
                            0,
                            bytemuck::cast_slice(&last_chunk[..end_item]),
                        );
                    }
                    for (buffer, data) in mid.iter().zip(data_iter) {
                        self.queue
                            .write_buffer(buffer, 0, bytemuck::cast_slice(data));
                    }
                }
            }
        }

        self.dirty_range = usize::MAX..0;
    }

    pub fn read_buffers(&self, mut f: impl FnMut(&Buffer, usize)) {
        if let [buffer] = &self.gpu_buffer.as_slice() {
            f(buffer, self.data.len());
        } else {
            let filled_buffer_len = self.max_buffer_size as usize / size_of::<T>();
            let num_buffers = self.data.len() / filled_buffer_len;
            let buffer_tail = self.data.len() % filled_buffer_len;
            for buffer in self.gpu_buffer.iter().take(num_buffers) {
                f(buffer, filled_buffer_len);
            }
            if buffer_tail > 0 {
                f(&self.gpu_buffer[num_buffers], buffer_tail);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_free_list_merge() {
        let mut fl = FreeList::new();

        // [10..20]
        fl.dealloc(10..20);
        assert_eq!(fl.by_start.get(&10), Some(&20));

        // [0..5, 10..20]
        fl.dealloc(0..5);
        assert_eq!(fl.by_start.len(), 2);

        // [0..5, 10..20, 25..30]
        fl.dealloc(25..30);
        assert_eq!(fl.by_start.len(), 3);

        // Merge next: [0..10, 10..20, 25..30] -> [0..20, 25..30]
        fl.dealloc(5..10);
        assert_eq!(fl.by_start.len(), 2);
        assert_eq!(fl.by_start.get(&0), Some(&20));

        // Merge both: [0..20, 20..25, 25..30] -> [0..30]
        fl.dealloc(20..25);
        assert_eq!(fl.by_start.len(), 1);
        assert_eq!(fl.by_start.get(&0), Some(&30));
    }

    #[test]
    fn test_free_list_alloc_split() {
        let mut fl = FreeList::new();
        fl.dealloc(0..10);
        fl.dealloc(20..30);
        fl.dealloc(40..50);

        // Alloc 5. Should pick 0..10
        let r1 = fl.alloc(5).unwrap();
        assert_eq!(r1, 0..5);
        // remains 5..10, 20..30, 40..50
        assert_eq!(fl.by_start.get(&5), Some(&10));

        // Alloc 10. Should pick 20..30
        let r2 = fl.alloc(10).unwrap();
        assert_eq!(r2, 20..30);
        // remains 5..10, 40..50
        assert_eq!(fl.by_start.len(), 2);
    }
}

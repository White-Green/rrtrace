use bytemuck::NoUninit;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::ops::{Index, IndexMut, Range};
use wgpu::{Buffer, BufferAddress, BufferDescriptor, BufferUsages, Device, Queue};

pub struct GpuSyncVec<T> {
    data: Vec<T>,
    dirty_range: Range<usize>,
    device: Device,
    queue: Queue,
    gpu_buffer: Vec<Buffer>,
    max_buffer_size: u64,
}

impl<T> Debug for GpuSyncVec<T>
where
    T: Debug,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.data).finish()
    }
}

impl<T> Index<usize> for GpuSyncVec<T> {
    type Output = T;

    fn index(&self, index: usize) -> &Self::Output {
        &self.data[index]
    }
}

impl<T> IndexMut<usize> for GpuSyncVec<T> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        let result = &mut self.data[index];
        self.dirty_range.start = self.dirty_range.start.min(index);
        self.dirty_range.end = self.dirty_range.end.max(index + 1);
        result
    }
}

impl<T> GpuSyncVec<T> {
    pub fn new(device: Device, queue: Queue, usage: BufferUsages) -> GpuSyncVec<T> {
        let max_buffer_size = device.limits().max_buffer_size;
        let gpu_buffer = device.create_buffer(&BufferDescriptor {
            label: None,
            size: (max_buffer_size / size_of::<T>() as u64).min(256) * size_of::<T>() as u64,
            usage,
            mapped_at_creation: false,
        });
        GpuSyncVec {
            data: Vec::new(),
            dirty_range: usize::MAX..0,
            device,
            queue,
            gpu_buffer: vec![gpu_buffer],
            max_buffer_size,
        }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn get(&self, index: usize) -> Option<&T> {
        self.data.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut T> {
        self.data.get_mut(index).inspect(|_| {
            self.dirty_range.start = self.dirty_range.start.min(index);
            self.dirty_range.end = self.dirty_range.end.max(index + 1);
        })
    }

    pub fn push(&mut self, value: T) {
        let updated_index = self.data.len();
        self.data.push(value);
        self.dirty_range.start = self.dirty_range.start.min(updated_index);
        self.dirty_range.end = self.dirty_range.end.max(updated_index + 1);
    }

    pub fn truncate(&mut self, len: usize) {
        if len < self.data.len() {
            self.data.truncate(len);
            self.dirty_range.end = self.dirty_range.end.min(len);
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

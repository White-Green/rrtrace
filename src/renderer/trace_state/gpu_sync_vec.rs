use bytemuck::NoUninit;
use std::fmt;
use std::fmt::{Debug, Formatter};
use std::ops::{Index, IndexMut, Range};
use wgpu::{Buffer, BufferAddress, BufferDescriptor, BufferUsages, BufferView, Device, Queue};

pub struct GpuSyncVec<T> {
    data: Vec<T>,
    dirty_range: Range<usize>,
    device: Device,
    queue: Queue,
    gpu_buffer: Buffer,
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
        let gpu_buffer = device.create_buffer(&BufferDescriptor {
            label: None,
            size: (size_of::<T>() * 256) as BufferAddress,
            usage,
            mapped_at_creation: false,
        });
        GpuSyncVec {
            data: Vec::new(),
            dirty_range: usize::MAX..0,
            device,
            queue,
            gpu_buffer,
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

    pub fn sync(&mut self)
    where
        T: NoUninit,
    {
        if self.dirty_range.start >= self.dirty_range.end {
            return;
        }

        let required_size = (self.data.len() * size_of::<T>()) as BufferAddress;
        if required_size > self.gpu_buffer.size() {
            self.gpu_buffer = self.device.create_buffer(&BufferDescriptor {
                label: None,
                size: required_size.next_power_of_two(),
                usage: self.gpu_buffer.usage(),
                mapped_at_creation: false,
            });
        }

        let dirty_data = &self.data[self.dirty_range.clone()];
        let offset = (self.dirty_range.start * size_of::<T>()) as BufferAddress;
        let bytes: &[u8] = bytemuck::cast_slice(dirty_data);
        self.queue.write_buffer(&self.gpu_buffer, offset, bytes);

        self.dirty_range = usize::MAX..0;
    }

    pub fn buffer(&self) -> &Buffer {
        &self.gpu_buffer
    }
}

use std::ffi::CString;

pub struct SharedMemory {
    ptr: *mut u8,
    name: CString,
    size: usize,
}

impl SharedMemory {
    pub unsafe fn open(name: CString, size: usize) -> SharedMemory {
        let fd = unsafe { libc::shm_open(name.as_ptr(), libc::O_RDWR, 0) };
        if fd < 0 {
            panic!("shm_open failed");
        }
        let memory = unsafe { libc::mmap(
            std::ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            fd,
            0,
        ) };
        if memory == libc::MAP_FAILED {
            panic!("mmap failed");
        }
        SharedMemory {
            ptr: memory as *mut u8,
            name,
            size,
        }
    }

    pub fn as_ptr<T>(&self) -> *mut T {
        self.ptr.cast()
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.size);
            libc::shm_unlink(self.name.as_ptr());
        }
    }
}

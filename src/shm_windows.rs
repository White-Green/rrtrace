use std::ffi::CString;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows_sys::Win32::System::Memory::{
    FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingA,
    UnmapViewOfFile,
};

pub struct SharedMemory {
    ptr: MEMORY_MAPPED_VIEW_ADDRESS,
    handle: HANDLE,
}

impl SharedMemory {
    pub unsafe fn open(name: CString, size: usize) -> SharedMemory {
        let handle =
            unsafe { OpenFileMappingA(FILE_MAP_ALL_ACCESS, 0, name.as_ptr() as *const u8) };
        if handle.is_null() {
            panic!("OpenFileMappingA failed with error {}", unsafe {
                GetLastError()
            });
        }

        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_ALL_ACCESS, 0, 0, size) };
        if ptr.Value.is_null() {
            unsafe { CloseHandle(handle) };
            panic!("MapViewOfFile failed");
        }

        SharedMemory { ptr, handle }
    }

    pub fn as_ptr<T>(&self) -> *mut T {
        self.ptr.Value.cast()
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.ptr);
            CloseHandle(self.handle);
        }
    }
}

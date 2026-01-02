use std::ffi::CString;
use windows_sys::Win32::System::Memory::*;
use windows_sys::Win32::Foundation::*;

pub struct SharedMemory {
    ptr: *mut u8,
    handle: HANDLE,
}

impl SharedMemory {
    pub unsafe fn open(name: CString, size: usize) -> SharedMemory {
        let handle = OpenFileMappingA(
            FILE_MAP_ALL_ACCESS,
            0,
            name.as_ptr() as *const u8,
        );
        if handle == 0 {
            panic!("OpenFileMappingA failed with error {}", unsafe { windows_sys::Win32::System::Diagnostics::Debug::GetLastError() });
        }

        let ptr = MapViewOfFile(
            handle,
            FILE_MAP_ALL_ACCESS,
            0,
            0,
            size,
        );
        if ptr.is_null() {
            CloseHandle(handle);
            panic!("MapViewOfFile failed");
        }

        SharedMemory {
            ptr: ptr as *mut u8,
            handle,
        }
    }

    pub fn as_ptr<T>(&self) -> *mut T {
        self.ptr.cast()
    }
}

impl Drop for SharedMemory {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.ptr as *const std::ffi::c_void);
            CloseHandle(self.handle);
        }
    }
}

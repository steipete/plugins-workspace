#[cfg(not(windows))]
compile_error!("This proof requires native Windows; a non-Windows pass is not valid evidence.");

#[cfg(windows)]
pub mod native {
    use std::{io, os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT},
        System::Threading::{
            CreateEventW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
            SYNCHRONIZATION_SYNCHRONIZE,
        },
    };

    pub struct Event(HANDLE);

    impl Event {
        pub fn create(name: &str) -> io::Result<Self> {
            let wide: Vec<u16> = std::ffi::OsStr::new(name)
                .encode_wide()
                .chain([0])
                .collect();
            // Each test owns a unique manual-reset event and closes its native handle.
            let handle = unsafe { CreateEventW(ptr::null(), 1, 0, wide.as_ptr()) };
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(handle))
            }
        }

        pub fn open(name: &str) -> io::Result<Self> {
            let wide: Vec<u16> = std::ffi::OsStr::new(name)
                .encode_wide()
                .chain([0])
                .collect();
            let handle = unsafe {
                OpenEventW(
                    EVENT_MODIFY_STATE | SYNCHRONIZATION_SYNCHRONIZE,
                    0,
                    wide.as_ptr(),
                )
            };
            if handle.is_null() {
                Err(io::Error::last_os_error())
            } else {
                Ok(Self(handle))
            }
        }

        pub fn signal(&self) -> io::Result<()> {
            if unsafe { SetEvent(self.0) } == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        }

        pub fn wait(&self, timeout_ms: u32) -> io::Result<()> {
            wait_handle(self.0, timeout_ms)
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub fn wait_handle(handle: HANDLE, timeout_ms: u32) -> io::Result<()> {
        match unsafe { WaitForSingleObject(handle, timeout_ms) } {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "native process/event wait timed out",
            )),
            _ => Err(io::Error::last_os_error()),
        }
    }
}

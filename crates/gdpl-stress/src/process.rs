use std::error::Error;
#[cfg(windows)]
pub struct Identity {
    handle: windows_sys::Win32::Foundation::HANDLE,
    created: u64,
}
#[cfg(windows)]
impl Identity {
    pub fn open(pid: u32) -> Result<Self, Box<dyn Error>> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{
                GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE,
            },
        };
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        let (mut created, mut exit, mut kernel, mut user) = (
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
            FILETIME::default(),
        );
        if unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) } == 0
        {
            let e = std::io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(e.into());
        }
        Ok(Self {
            handle,
            created: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        })
    }
    pub fn verify(&self, pid: u32) -> Result<(), Box<dyn Error>> {
        use windows_sys::Win32::{
            Foundation::WAIT_TIMEOUT, System::Threading::WaitForSingleObject,
        };
        if unsafe { WaitForSingleObject(self.handle, 0) } != WAIT_TIMEOUT {
            return Err("target has exited or cannot be monitored".into());
        }
        let current = Self::open(pid)?;
        if current.created != self.created
            || unsafe { WaitForSingleObject(current.handle, 0) } != WAIT_TIMEOUT
        {
            return Err("target PID identity changed".into());
        }
        Ok(())
    }
    pub fn creation_time(&self) -> Option<u64> {
        Some(self.created)
    }
}
#[cfg(windows)]
impl Drop for Identity {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}
#[cfg(not(windows))]
pub struct Identity;
#[cfg(not(windows))]
impl Identity {
    pub fn open(_: u32) -> Result<Self, Box<dyn Error>> {
        Ok(Self)
    }
    pub fn verify(&self, _: u32) -> Result<(), Box<dyn Error>> {
        Ok(())
    }
    pub fn creation_time(&self) -> Option<u64> {
        None
    }
}

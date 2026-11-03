use std::error::Error;
#[cfg(windows)]
use std::io;

#[cfg(windows)]
pub struct ProcessHandle {
    handle: windows_sys::Win32::Foundation::HANDLE,
    creation_time: u64,
}

#[cfg(windows)]
impl ProcessHandle {
    pub fn open(pid: u32) -> Result<Self, Box<dyn Error>> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, FILETIME},
            System::Threading::{
                GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE,
            },
        };
        // A held process handle pins the kernel process object, so checks do not silently follow a reused PID.
        let handle = unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                0,
                pid,
            )
        };
        if handle.is_null() {
            return Err(io::Error::last_os_error().into());
        }
        let mut created = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        if unsafe { GetProcessTimes(handle, &mut created, &mut exit, &mut kernel, &mut user) } == 0
        {
            let error = io::Error::last_os_error();
            unsafe {
                CloseHandle(handle);
            }
            return Err(error.into());
        }
        Ok(Self {
            handle,
            creation_time: ((created.dwHighDateTime as u64) << 32) | created.dwLowDateTime as u64,
        })
    }
    pub fn exited(&self) -> Result<bool, Box<dyn Error>> {
        use windows_sys::Win32::{
            Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT},
            System::Threading::WaitForSingleObject,
        };
        match unsafe { WaitForSingleObject(self.handle, 0) } {
            WAIT_OBJECT_0 => Ok(true),
            WAIT_TIMEOUT => Ok(false),
            _ => Err(io::Error::last_os_error().into()),
        }
    }
    pub fn creation_time(&self) -> Option<u64> {
        Some(self.creation_time)
    }
    pub fn verify_identity(&self, pid: u32) -> Result<(), Box<dyn Error>> {
        if self.exited()? {
            return Err("target exited before identity verification".into());
        }
        let current = Self::open(pid)?;
        if current.creation_time != self.creation_time || current.exited()? || self.exited()? {
            return Err("target PID identity changed or process exited".into());
        }
        Ok(())
    }
}
#[cfg(windows)]
impl Drop for ProcessHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(not(windows))]
pub struct ProcessHandle;
#[cfg(not(windows))]
impl ProcessHandle {
    pub fn open(_: u32) -> Result<Self, Box<dyn Error>> {
        Err("unsupported platform".into())
    }
    pub fn exited(&self) -> Result<bool, Box<dyn Error>> {
        Err("unsupported platform".into())
    }
    pub fn creation_time(&self) -> Option<u64> {
        None
    }
    pub fn verify_identity(&self, _: u32) -> Result<(), Box<dyn Error>> {
        Err("unsupported platform".into())
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::ProcessHandle;
    #[test]
    fn detects_identity_mismatch() {
        let current = ProcessHandle::open(std::process::id()).unwrap();
        assert!(current.verify_identity(std::process::id()).is_ok());
        assert!(current.verify_identity(0).is_err());
    }
}

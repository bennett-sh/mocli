use anyhow::Result;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
    },
    core::w,
};

/// Held for the lifetime of the process.
pub struct Instance(HANDLE);

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the handle came from `CreateMutexW` and is closed once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Returns `None` if another instance runs in this Windows session.
pub fn acquire() -> Result<Option<Instance>> {
    // SAFETY: the name is a static string; the last error is read immediately.
    unsafe {
        let handle = CreateMutexW(None, false, w!(r"Local\mocli-single-instance"))?;
        let existed = GetLastError() == ERROR_ALREADY_EXISTS;
        let instance = Instance(handle);
        Ok((!existed).then_some(instance))
    }
}

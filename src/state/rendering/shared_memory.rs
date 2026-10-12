//! One named transport resource. Drop is necessary to unlink the OS name on errors and unwinding.
use std::{ffi::CString, fs::File};

pub(crate) struct SharedMemoryImage {
    pub name: CString,
    pub file: File,
    pub bytes: usize,
}

impl Drop for SharedMemoryImage {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if self.file.metadata().is_ok_and(|metadata| metadata.nlink() == 0) { return; }
            let _ = rustix::shm::unlink(self.name.as_c_str()); // clean up failed or unconsumed transfers
        }
    }
}

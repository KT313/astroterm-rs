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

#[cfg(feature = "memory-diagnostics")]
impl crate::cache::ReportBuffers for SharedMemoryImage {
    fn report_buffers(&self, sink: &mut dyn crate::cache::BufferSink) {
        if sink.enter("rgb_object", std::mem::size_of::<Self>()) {
            let owner = sink.set_owner(crate::cache::Owner::External);
            sink.borrowed(self.bytes, 1, "kernel-owned RGB extent exposed through a file handle; not Rust heap or a client mapping");
            sink.unknown("OS allocation overhead and resident pages are not measured");
            sink.set_owner(owner);
            sink.unknown("file handle and name allocator capacity excluded");
            sink.leave();
        }
    }
}

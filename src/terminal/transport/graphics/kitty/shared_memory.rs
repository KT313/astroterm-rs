//! Copy existing RGB into a named POSIX object; no mappings or terminal-input reader are needed in the frame loop.
use std::{io, fmt::Write as _};
use crate::state::SharedMemoryImage;
use ratatui_image::picker::cap_parser::Parser;

#[cfg(unix)]
pub(crate) fn create_shared_image(rgb: &[u8]) -> io::Result<SharedMemoryImage> {
    use std::{ffi::CString, fs::File, io::Write, sync::{OnceLock, atomic::{AtomicU64, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
    use rustix::shm::{self, OFlags, Mode};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    static START: OnceLock<u64> = OnceLock::new();
    let start = START.get_or_init(|| (SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos() as u64) ^ u64::from(std::process::id()).rotate_left(32));
    for _ in 0..8 {
        let number = NEXT.fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1)).map_err(|_| io::Error::other("shared-memory identifier exhausted"))?;
        let name = CString::new(format!("/astroterm-{:x}", start ^ number)).unwrap();
        let fd = match shm::open(name.as_c_str(), OFlags::CREATE | OFlags::EXCL | OFlags::RDWR, Mode::RUSR | Mode::WUSR) {
            Ok(fd) => fd,
            Err(rustix::io::Errno::EXIST) => continue,                                // never overwrite or unlink an object we did not create
            Err(error) => return Err(error.into()),
        };
        let mut object = SharedMemoryImage { name, file: File::from(fd), bytes: rgb.len() };
        object.file.set_len(rgb.len() as u64)?;
        object.file.write_all(rgb)?;                                                 // preserve the completed RGB allocation; this prototype adds one copy
        return Ok(object);
    }
    Err(io::Error::other("cannot obtain a unique shared-memory name"))
}

#[cfg(not(unix))]
pub(crate) fn create_shared_image(_rgb: &[u8]) -> io::Result<SharedMemoryImage> {
    Err(io::Error::other("shared-memory transport is unavailable on this platform"))
}

pub(crate) fn encode_shared_upload(object: &SharedMemoryImage, dimensions: (u32, u32), id: u32, query: bool, tmux: bool, output: &mut String) {
    output.clear();
    let (start, escape, end) = Parser::tmux_start_escape_end(tmux);
    let action = if query { "q" } else { "t" };
    let quiet = if query { 0 } else { 2 };
    write!(output, "{start}{escape}_Gi={id},a={action},f=24,t=s,s={},v={},S={},q={quiet};", dimensions.0, dimensions.1, object.bytes).unwrap();
    base64_simd::STANDARD.encode_append(object.name.to_bytes(), output);
    write!(output, "{escape}\\{end}").unwrap();
}

#[cfg(unix)]
fn was_consumed(object: &SharedMemoryImage) -> io::Result<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(object.file.metadata()?.nlink() == 0) // check the original inode, even if another object reused its name
}

pub(crate) fn wait_for_shared_consumption(object: &SharedMemoryImage) -> io::Result<bool> {
    #[cfg(unix)]
    {
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_millis(crate::constants::KITTY_SHARED_MEMORY_TIMEOUT_MS);
        loop {
            if was_consumed(object)? { return Ok(true); }
            if Instant::now() >= deadline { return Ok(false); }
            std::thread::sleep(Duration::from_millis(crate::constants::KITTY_SHARED_MEMORY_POLL_MS.max(1)));
        }
    }
    #[cfg(not(unix))]
    { let _ = object; Ok(false) }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn named_rgb_is_exact_and_remains_readable_until_consumption() {
        let rgb = vec![12, 34, 56, 78, 90, 123];
        let object = create_shared_image(&rgb).unwrap();
        let mut reader = std::fs::File::from(rustix::shm::open(object.name.as_c_str(), rustix::shm::OFlags::RDONLY, rustix::shm::Mode::empty()).unwrap());
        let mut actual = Vec::new(); reader.read_to_end(&mut actual).unwrap();
        assert_eq!(actual, rgb); assert!(!was_consumed(&object).unwrap());
        let mut command = String::new();
        encode_shared_upload(&object, (2, 1), 45, false, false, &mut command);
        assert!(command.contains("a=t,f=24,t=s,s=2,v=1,S=6,q=2;"));
        let payload = command.split_once(';').unwrap().1.trim_end_matches("\x1b\\");
        assert_eq!(base64_simd::STANDARD.decode_to_vec(payload).unwrap(), object.name.to_bytes());
        rustix::shm::unlink(object.name.as_c_str()).unwrap();
        assert!(wait_for_shared_consumption(&object).unwrap());
    }

    #[test]
    fn drop_cleans_unconsumed_names_and_unlinked_objects_do_not_delete_replacements() {
        let object = create_shared_image(&[1, 2, 3]).unwrap();
        let name = object.name.clone(); drop(object);
        assert_eq!(rustix::shm::open(name.as_c_str(), rustix::shm::OFlags::RDONLY, rustix::shm::Mode::empty()).unwrap_err(), rustix::io::Errno::NOENT);
        let old = create_shared_image(&[1, 2, 3]).unwrap();
        let name = old.name.clone(); rustix::shm::unlink(name.as_c_str()).unwrap();
        let fd = rustix::shm::open(name.as_c_str(), rustix::shm::OFlags::CREATE | rustix::shm::OFlags::EXCL | rustix::shm::OFlags::RDWR, rustix::shm::Mode::RUSR | rustix::shm::Mode::WUSR).unwrap();
        let replacement = SharedMemoryImage { name, file: fd.into(), bytes: 0 };
        drop(old); // its inode is already unlinked; the replacement name belongs to another object
        assert!(!was_consumed(&replacement).unwrap());
    }
}

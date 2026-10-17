//! Huge pages for the heap. The star tiles take hundreds of megabytes, and zero-filling them 4 KB page by page on
//! first touch costs most of the first frame. glibc's malloc marks its memory for transparent huge pages only when
//! the `glibc.malloc.hugetlb=1` tunable is set when the process starts, so the program restarts itself once with
//! it, where the system allows transparent huge pages. Measurements: `dev/first-frame-page-faults/REPORT.md`.
use std::ffi::{OsStr, OsString};

/// The tunable that makes glibc's malloc request transparent huge pages for its memory.
const HUGE_PAGE_TUNABLE: &str = "glibc.malloc.hugetlb=1";

/// Restart this process once, with the same arguments, using glibc's huge-page malloc when the system allows
/// transparent huge pages. Returns, and the program runs on unchanged, on other systems, when transparent huge
/// pages are disabled, when the user already chose a `glibc.malloc.hugetlb` value (`=0` opts out), or when the
/// restart fails. Call it first in `main`, before any thread or terminal state exists.
pub(crate) fn restart_with_huge_pages_if_allowed() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        use std::os::unix::process::CommandExt;
        let Ok(mode) = std::fs::read_to_string("/sys/kernel/mm/transparent_hugepage/enabled") else { return };
        let Some(tunables) = add_huge_page_tunable(std::env::var_os("GLIBC_TUNABLES").as_deref(), &mode) else { return };

        // Replace this process image: same process ID, terminal, arguments and environment plus the tunable.
        let mut arguments = std::env::args_os();
        let mut restart = std::process::Command::new("/proc/self/exe");
        if let Some(program) = arguments.next() { restart.arg0(program); }
        let _failure = restart.args(arguments).env("GLIBC_TUNABLES", tunables).exec();         // returns only if the restart failed: run on as before
    }
}

/// The `GLIBC_TUNABLES` value with huge-page malloc added, or `None` when nothing should change: the kernel's
/// transparent-huge-page mode (`/sys/kernel/mm/transparent_hugepage/enabled`, the selected mode in brackets, e.g.
/// `always [madvise] never`) is `never` or unreadable, or `current` already sets `glibc.malloc.hugetlb`, which also
/// keeps the restarted process from restarting again.
#[cfg_attr(not(all(target_os = "linux", target_env = "gnu")), allow(dead_code))]
fn add_huge_page_tunable(current: Option<&OsStr>, transparent_huge_page_mode: &str) -> Option<OsString> {
    let selected = transparent_huge_page_mode.split_whitespace().find(|mode| mode.starts_with('[') && mode.ends_with(']'))?;
    if selected == "[never]" { return None; }
    let Some(current) = current.filter(|tunables| !tunables.is_empty()) else { return Some(HUGE_PAGE_TUNABLE.into()) };
    let chosen = current.as_encoded_bytes().split(|&byte| byte == b':').any(|tunable| tunable.starts_with(b"glibc.malloc.hugetlb="));
    if chosen { return None; }
    let mut tunables = current.to_os_string();
    tunables.push(":");
    tunables.push(HUGE_PAGE_TUNABLE);
    Some(tunables)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_pages_are_added_where_the_kernel_allows_them() {
        for mode in ["always [madvise] never\n", "[always] madvise never", "always defer defer+madvise [madvise] never"] {
            assert_eq!(add_huge_page_tunable(None, mode), Some(OsString::from("glibc.malloc.hugetlb=1")), "{mode}");
        }
        for mode in ["always madvise [never]\n", "", "madvise"] {
            assert_eq!(add_huge_page_tunable(None, mode), None, "{mode:?}");                  // disabled or unreadable: no restart
        }
    }

    #[test]
    fn other_tunables_are_kept_and_a_chosen_huge_page_setting_is_respected() {
        let mode = "always [madvise] never";
        let tunables = |value: &str| Some(OsString::from(value));
        assert_eq!(add_huge_page_tunable(Some(OsStr::new("")), mode), tunables("glibc.malloc.hugetlb=1"));
        assert_eq!(add_huge_page_tunable(Some(OsStr::new("glibc.malloc.arena_max=2")), mode), tunables("glibc.malloc.arena_max=2:glibc.malloc.hugetlb=1"));
        for chosen in ["glibc.malloc.hugetlb=1", "glibc.malloc.hugetlb=0", "glibc.malloc.arena_max=2:glibc.malloc.hugetlb=2"] {
            assert_eq!(add_huge_page_tunable(Some(OsStr::new(chosen)), mode), None, "{chosen}");   // the restarted process, or the user's choice
        }
    }
}

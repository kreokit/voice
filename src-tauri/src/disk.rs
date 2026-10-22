//! Free disk space (from KreoKit Clips' `batch.rs`), checked before large writes.

use std::path::Path;

/// Fails with a readable message when writing `needed` bytes into `dir` would leave less
/// than `margin` free. When the free space can't be read, the write is attempted anyway.
pub fn ensure_free(dir: &Path, needed: u64, margin: u64) -> Result<(), String> {
    match free_bytes(dir) {
        Some(free) if free < needed.saturating_add(margin) => Err(format!(
            "Not enough free disk space: this needs about {} and {} is free (keeping {} spare). Free up some space, then try again.",
            gigabytes(needed),
            gigabytes(free),
            gigabytes(margin)
        )),
        _ => Ok(()),
    }
}

/// "0.7 GB", "12 GB".
pub fn gigabytes(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb < 10.0 {
        format!("{gb:.1} GB")
    } else {
        format!("{gb:.0} GB")
    }
}

/// Bytes available to this user on the volume holding `path`.
#[cfg(unix)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `stats` a valid out pointer.
    let result = unsafe { libc::statvfs(c.as_ptr(), &mut stats) };
    (result == 0).then(|| stats.f_bavail as u64 * stats.f_frsize as u64)
}

#[cfg(windows)]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetDiskFreeSpaceExW(
            dir: *const u16,
            available: *mut u64,
            total: *mut u64,
            free: *mut u64,
        ) -> i32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut available = 0u64;
    // SAFETY: `wide` is NUL-terminated; null pointers are allowed for the totals.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut available,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    (ok != 0).then_some(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_free_space_and_refuses_impossible_writes() {
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir).is_some_and(|b| b > 0));
        assert!(ensure_free(&dir, 1, 0).is_ok());
        let err = ensure_free(&dir, u64::MAX / 2, 1 << 30).unwrap_err();
        assert!(err.starts_with("Not enough free disk space"), "{err}");
    }

    #[test]
    fn formats_gigabytes() {
        assert_eq!(gigabytes(691_560_000), "0.7 GB");
        assert_eq!(gigabytes(12_400_000_000), "12 GB");
    }
}

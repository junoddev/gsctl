//! File access that doesn't trust the filesystem: no following a symlink at
//! the final path component, no blocking on FIFOs, and a size cap.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

/// `O_NOFOLLOW` from Linux's `fcntl.h` (no libc dependency): the open fails
/// if the last path component is a symlink.
#[cfg(any(target_arch = "aarch64", target_arch = "arm", target_arch = "powerpc", target_arch = "powerpc64"))]
pub const O_NOFOLLOW: i32 = 0o100000;
#[cfg(not(any(target_arch = "aarch64", target_arch = "arm", target_arch = "powerpc", target_arch = "powerpc64")))]
pub const O_NOFOLLOW: i32 = 0o400000;

/// `O_NONBLOCK`, so opening a FIFO doesn't wait for a writer.
#[cfg(any(target_arch = "mips", target_arch = "mips64"))]
const O_NONBLOCK: i32 = 0x80;
#[cfg(target_arch = "sparc64")]
const O_NONBLOCK: i32 = 0x4000;
#[cfg(not(any(target_arch = "mips", target_arch = "mips64", target_arch = "sparc64")))]
const O_NONBLOCK: i32 = 0o4000;

/// The most `read_small` reads.
pub const MAX_FILE_SIZE: u64 = 1 << 20;

/// The text of a regular file of at most [`MAX_FILE_SIZE`] bytes, or `None`
/// if `path` is a symlink, a directory, a FIFO or other special file, too
/// big, unreadable or not UTF-8.
pub fn read_small(path: &Path) -> Option<String> {
    if !fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    let f = OpenOptions::new().read(true).custom_flags(O_NOFOLLOW | O_NONBLOCK).open(path).ok()?;
    let meta = f.metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_FILE_SIZE {
        return None;
    }
    let mut bytes = Vec::new();
    f.take(MAX_FILE_SIZE + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() as u64 > MAX_FILE_SIZE {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// Open `path` for appending, creating it with mode 0600, and refusing a
/// symlink at `path`.
pub fn open_append_private(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).mode(0o600).custom_flags(O_NOFOLLOW).open(path)
}

/// Replace `path` with `contents`, mode 0600: write a temp file in the same
/// directory, then rename it over `path`, so a reader never sees half a file.
/// Refuses if `path` is a symlink or not a regular file.
pub fn replace_private(path: &Path, contents: &[u8]) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => {
            return Err(io::Error::other("refusing to write through a symlink"));
        }
        Ok(m) if !m.is_file() => return Err(io::Error::other("not a regular file")),
        _ => {}
    }
    let name = path.file_name().ok_or_else(|| io::Error::other("no file name"))?.to_string_lossy();
    let tmp = path.with_file_name(format!(".{name}.tmp-{}", std::process::id()));
    let write = || -> io::Result<()> {
        let mut f = OpenOptions::new().write(true).create_new(true).mode(0o600).custom_flags(O_NOFOLLOW).open(&tmp)?;
        f.write_all(contents)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    };
    write().inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_small_regular_files() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        fs::write(&file, "hello").unwrap();
        assert_eq!(read_small(&file).as_deref(), Some("hello"));

        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(read_small(&link), None);
        assert_eq!(read_small(dir.path()), None);

        let big = dir.path().join("big.txt");
        fs::write(&big, vec![b'x'; MAX_FILE_SIZE as usize + 1]).unwrap();
        assert_eq!(read_small(&big), None);
    }

    #[test]
    fn append_refuses_symlinks_and_creates_private_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal");
        open_append_private(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);

        let target = dir.path().join("target");
        fs::write(&target, "").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_append_private(&link).is_err());
    }
}

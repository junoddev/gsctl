//! File access that doesn't trust the filesystem: no following a symlink at
//! the final path component, no blocking on FIFOs, and a size cap.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
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

/// [`read_small`], or why `path` can't be read that way.
pub fn read_small_or_why(path: &Path) -> Result<String, String> {
    if let Some(text) = read_small(path) {
        return Ok(text);
    }
    Err(match fs::symlink_metadata(path) {
        Err(e) => e.to_string(),
        Ok(m) if m.file_type().is_symlink() => {
            "it is a symlink, which gsctl doesn't follow; pass the file it points to".into()
        }
        Ok(m) if !m.is_file() => "not a regular file".into(),
        Ok(m) if m.len() > MAX_FILE_SIZE => format!("larger than {} bytes", MAX_FILE_SIZE),
        Ok(_) => "unreadable, or not UTF-8".into(),
    })
}

/// Open `path` for appending, creating it with mode 0600, and refusing a
/// symlink at `path`. An existing file, and the directory it's in, are
/// tightened to 0600 and 0700 if we own them (an older gsctl, a umask or a
/// copy may have left them readable by others).
pub fn open_append_private(path: &Path) -> io::Result<File> {
    let f = OpenOptions::new().create(true).append(true).mode(0o600).custom_flags(O_NOFOLLOW).open(path)?;
    tighten(&f, 0o600)?;
    if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
        // O_NOFOLLOW: a symlinked directory is left alone. Opening a
        // directory read-only is allowed; fchmod works on its descriptor.
        if let Ok(d) = OpenOptions::new().read(true).custom_flags(O_NOFOLLOW | O_NONBLOCK).open(dir)
            && d.metadata()?.is_dir()
        {
            tighten(&d, 0o700)?;
        }
    }
    Ok(f)
}

/// `fchmod` the open file `f` to `mode` if its permission bits differ and
/// we own it. Works on the descriptor, so a path swapped since the open
/// can't redirect it.
fn tighten(f: &File, mode: u32) -> io::Result<()> {
    let meta = f.metadata()?;
    if meta.mode() & 0o7777 == mode || Some(meta.uid()) != euid() {
        return Ok(());
    }
    f.set_permissions(fs::Permissions::from_mode(mode))
}

/// This process's effective uid, from `/proc/self/status` (no libc
/// dependency); `None` if it can't be read.
fn euid() -> Option<u32> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find_map(|l| l.strip_prefix("Uid:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
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

    fn mode(p: &Path) -> u32 {
        fs::metadata(p).unwrap().permissions().mode() & 0o7777
    }

    #[test]
    fn append_tightens_an_existing_file_and_its_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("gsctl");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        let path = dir.join("journal");
        fs::write(&path, "old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        open_append_private(&path).unwrap();
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(fs::read_to_string(&path).unwrap(), "old\n");
    }

    #[test]
    fn append_leaves_a_symlinked_dir_alone() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        fs::create_dir(&real).unwrap();
        fs::set_permissions(&real, fs::Permissions::from_mode(0o755)).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        open_append_private(&link.join("journal")).unwrap();
        assert_eq!(mode(&real), 0o755, "not followed through the symlink");
        assert_eq!(mode(&real.join("journal")), 0o600);
    }

    #[test]
    fn euid_is_known() {
        let ours = fs::metadata(tempfile::tempdir().unwrap().path()).unwrap().uid();
        assert_eq!(euid(), Some(ours));
    }
}

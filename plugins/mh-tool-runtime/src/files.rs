//! Capability-relative regular-file opens shared by tools and prompt sources.
use cap_std::fs::{Dir, File};
use std::{
    io,
    path::{Component, Path},
};
pub fn open_regular_file(dir: &Dir, path: &Path) -> io::Result<File> {
    if path.is_absolute()
        || !path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "source path must remain inside its directory",
        ));
    }
    #[cfg(unix)]
    let file = {
        use std::{
            ffi::CString,
            os::{
                fd::{AsRawFd, FromRawFd},
                unix::ffi::OsStrExt,
            },
        };
        let parent = dir.open_dir(
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?;
        let name = CString::new(
            path.file_name()
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "source must name a regular file",
                    )
                })?
                .as_bytes(),
        )
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "source contains NUL"))?;
        // The parent capability resolves intermediate components; final openat
        // cannot follow a swapped symlink or wait for a FIFO writer.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // Successful openat transfers exactly one owned descriptor to File.
        File::from_std(unsafe { std::fs::File::from_raw_fd(fd) })
    };
    #[cfg(not(unix))]
    let file = dir.open(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{GetFileType, FILE_TYPE_DISK};
        // Metadata alone can classify DOS devices as files on Windows.
        if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "source must be a regular disk file",
            ));
        }
    }
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "source must be a regular file",
        ));
    }
    Ok(file)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_directories_and_reads_regular_files() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "inside").unwrap();
        let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        assert!(open_regular_file(&dir, Path::new(".")).is_err());
        assert!(open_regular_file(&dir, Path::new("file")).is_ok());
        assert!(open_regular_file(&dir, Path::new("../outside")).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn rejects_windows_devices() {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        assert!(open_regular_file(&dir, Path::new("NUL")).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn rejects_final_symlinks_even_inside_scope() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("file"), "inside").unwrap();
        std::os::unix::fs::symlink("file", root.path().join("link")).unwrap();
        let dir = Dir::open_ambient_dir(root.path(), cap_std::ambient_authority()).unwrap();
        assert!(open_regular_file(&dir, Path::new("link")).is_err());
    }
}

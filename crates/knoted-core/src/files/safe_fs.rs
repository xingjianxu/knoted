//! Linux FD-relative, no-follow read access. Paths never authorize a read on
//! their own: each component is opened relative to an already held directory.
use rustix::fs::{self, Mode, OFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::CString,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    path::{Component, Path},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpaqueHandle {
    pub kind: i32,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Identity {
    pub device: u64,
    pub inode: u64,
    pub handle: Option<OpaqueHandle>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub identity: Identity,
    pub size: u64,
    pub mtime_ns: i64,
    pub ctime_ns: i64,
    pub nlink: u64,
}
const DIRECTORY: OFlags = OFlags::RDONLY
    .union(OFlags::DIRECTORY)
    .union(OFlags::NOFOLLOW)
    .union(OFlags::CLOEXEC);

pub fn open_root(path: &Path) -> std::io::Result<OwnedFd> {
    if !path.is_absolute() {
        return Err(std::io::ErrorKind::InvalidInput.into());
    }
    let mut fd = fs::open("/", DIRECTORY, Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => fd = fs::openat(&fd, name, DIRECTORY, Mode::empty())?,
            _ => return Err(std::io::ErrorKind::InvalidInput.into()),
        }
    }
    Ok(fd)
}
pub fn open_directory(root: impl AsFd, path: &Path) -> std::io::Result<OwnedFd> {
    let mut fd = fs::openat(root, ".", DIRECTORY, Mode::empty())?;
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(std::io::ErrorKind::InvalidInput.into());
        };
        fd = fs::openat(&fd, name, DIRECTORY, Mode::empty())?;
    }
    Ok(fd)
}
/// O_PATH avoids opening devices/FIFOs for IO. Only regular-file descriptors
/// are passed on as observations. T44 must reopen RDONLY|NONBLOCK and recheck.
pub fn open_entry(parent: impl AsFd, name: &std::ffi::CStr) -> std::io::Result<OwnedFd> {
    Ok(fs::openat(
        parent,
        name,
        OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )?)
}
pub fn metadata(fd: impl AsFd) -> std::io::Result<Metadata> {
    let stat = fs::fstat(&fd)?;
    let ns = |s: i64, n: i64| {
        s.checked_mul(1_000_000_000)
            .and_then(|v| v.checked_add(n))
            .ok_or(std::io::ErrorKind::InvalidData)
    };
    Ok(Metadata {
        identity: Identity {
            device: stat.st_dev,
            inode: stat.st_ino,
            handle: opaque_handle(&fd),
        },
        size: stat.st_size as u64,
        mtime_ns: ns(stat.st_mtime, stat.st_mtime_nsec as i64)?,
        ctime_ns: ns(stat.st_ctime, stat.st_ctime_nsec as i64)?,
        nlink: stat.st_nlink,
    })
}
/// Linux name_to_handle_at(AT_EMPTY_PATH) obtains a non-parent opaque handle
/// from the actual FD. Unsupported filesystems/permissions return None: never
/// substitute ctime/btime as an incarnation proof. Mount IDs are not durable.
fn opaque_handle(fd: impl AsFd) -> Option<OpaqueHandle> {
    #[repr(C)]
    struct Handle {
        len: u32,
        kind: i32,
        bytes: [u8; 128],
    }
    let mut handle = Handle {
        len: 128,
        kind: 0,
        bytes: [0; 128],
    };
    let mut mount = 0;
    let empty = CString::new("").unwrap();
    // SAFETY: Handle has the kernel file_handle layout followed by 128 writable
    // bytes; len is its capacity. All pointers live for the synchronous syscall.
    let result = unsafe {
        libc::name_to_handle_at(
            fd.as_fd().as_raw_fd(),
            empty.as_ptr(),
            (&mut handle as *mut Handle).cast(),
            &mut mount,
            libc::AT_EMPTY_PATH,
        )
    };
    if result != 0 || handle.len > 128 {
        return None;
    }
    Some(OpaqueHandle {
        kind: handle.kind,
        bytes: handle.bytes[..handle.len as usize].to_vec(),
    })
}

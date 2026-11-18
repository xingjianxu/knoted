//! Linux FD-relative, no-follow read access. Paths never authorize a read on
//! their own: each component is opened relative to an already held directory.
use rustix::fs::{self, AtFlags, FileType, Mode, OFlags, ResolveFlags, StatxFlags};
use serde::{Deserialize, Serialize};
use std::{
    ffi::{CString, OsString},
    io,
    os::fd::{AsFd, AsRawFd, OwnedFd},
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
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
/// Refuse storage roots that overlap the external, read-only source tree.
pub(crate) fn open_collection_root(
    ctx: &crate::context::AppContext,
    root: &Path,
) -> std::io::Result<OwnedFd> {
    let parent = root.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    let name = root.file_name().ok_or(std::io::ErrorKind::InvalidInput)?;
    let directory = open_collection_directory(&open_root(parent)?, Path::new(name))?;
    if let Some(source) = ctx.files().root.as_deref() {
        let source = source.canonicalize()?;
        let root = root.canonicalize()?;
        if root.starts_with(&source) || source.starts_with(&root) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
        let source_fd = open_root(&source)?;
        let mounts = std::fs::read("/proc/self/mountinfo")?;
        if mounted_paths_overlap(
            &mounts,
            &root,
            mount_id(&directory)?,
            &source,
            mount_id(&source_fd)?,
        )? {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(directory)
}

fn mount_id(dir: impl AsFd) -> io::Result<u64> {
    let stat = fs::statx(dir, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;
    if !StatxFlags::from_bits_retain(stat.stx_mask).contains(StatxFlags::MNT_ID) {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(stat.stx_mnt_id)
}

struct MountRoot<'a> {
    id: &'a [u8],
    device: &'a [u8],
    root: PathBuf,
    target: PathBuf,
}

fn decode_mount_path(mut value: &[u8]) -> io::Result<PathBuf> {
    let mut path = Vec::with_capacity(value.len());
    while let Some((&byte, rest)) = value.split_first() {
        value = rest;
        if byte != b'\\' {
            path.push(byte);
            continue;
        }
        let (escape, tail) = value
            .split_at_checked(3)
            .ok_or(io::ErrorKind::InvalidData)?;
        path.push(match escape {
            b"040" => b' ',
            b"011" => b'\t',
            b"012" => b'\n',
            b"134" => b'\\',
            _ => return Err(io::ErrorKind::InvalidData.into()),
        });
        value = tail;
    }
    let decoded = PathBuf::from(OsString::from_vec(path));
    if !decoded.is_absolute() {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(decoded)
}

fn mounted_paths_overlap(
    info: &[u8],
    storage: &Path,
    storage_id: u64,
    source: &Path,
    source_id: u64,
) -> io::Result<bool> {
    let mounts = info
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            let mut fields = line.split(|byte| *byte == b' ');
            let mut field = || fields.next().ok_or(io::ErrorKind::InvalidData);
            let id = field()?;
            field()?; // Parent mount ID.
            Ok(MountRoot {
                id,
                device: field()?,
                root: decode_mount_path(field()?)?,
                target: decode_mount_path(field()?)?,
            })
        })
        .collect::<io::Result<Vec<_>>>()?;
    let by_id = |id: u64| {
        let id = id.to_string();
        mounts
            .iter()
            .find(|entry| entry.id == id.as_bytes())
            .ok_or(io::ErrorKind::InvalidData)
    };
    let storage_mount = by_id(storage_id)?;
    let source_mount = by_id(source_id)?;
    let storage_path = storage_mount.root.join(
        storage
            .strip_prefix(&storage_mount.target)
            .map_err(|_| io::ErrorKind::InvalidData)?,
    );
    let source_path = source_mount.root.join(
        source
            .strip_prefix(&source_mount.target)
            .map_err(|_| io::ErrorKind::InvalidData)?,
    );
    let overlaps = |entry: &MountRoot<'_>, path: &Path| {
        entry.device == storage_mount.device
            && (storage_path.starts_with(path) || path.starts_with(&storage_path))
    };
    // A different spelling can address the same device subtree via a bind
    // mount. Include volumes mounted below the configured read-only root.
    Ok(overlaps(source_mount, &source_path)
        || mounts.iter().any(|entry| {
            entry.target != source
                && entry.target.starts_with(source)
                && overlaps(entry, &entry.root)
        }))
}

/// An explicitly configured parent can live on a mounted volume. A collection
/// root and its children must be ordinary directories within that volume.
pub(crate) fn open_collection_directory(dir: impl AsFd, name: &Path) -> std::io::Result<OwnedFd> {
    Ok(fs::openat2(
        dir,
        name,
        DIRECTORY,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_XDEV | ResolveFlags::NO_SYMLINKS,
    )?)
}

pub(crate) fn unlink_regular(dir: impl AsFd, name: &str) -> std::io::Result<bool> {
    let metadata = match fs::statat(&dir, name, fs::AtFlags::SYMLINK_NOFOLLOW) {
        Ok(metadata) => metadata,
        Err(rustix::io::Errno::NOENT) => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if FileType::from_raw_mode(metadata.st_mode) != FileType::RegularFile || metadata.st_nlink != 1
    {
        return Err(std::io::ErrorKind::InvalidData.into());
    }
    match fs::unlinkat(dir, name, fs::AtFlags::empty()) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::NOENT) => Ok(false),
        Err(error) => Err(error.into()),
    }
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

#[cfg(test)]
mod collection_tests {
    use super::*;

    #[test]
    fn t67_8_mount_roots_detect_a_bound_source_descendant() {
        let info = b"41 0 0:35 /@ / rw - btrfs /dev/root rw\n60 41 0:35 /@/source/nested /storage rw - btrfs /dev/root rw\n61 41 0:35 /@/unrelated /other rw - btrfs /dev/root rw\n";
        assert!(
            mounted_paths_overlap(
                info,
                Path::new("/storage/stage"),
                60,
                Path::new("/source"),
                41
            )
            .unwrap()
        );
        assert!(
            !mounted_paths_overlap(
                info,
                Path::new("/other/stage"),
                61,
                Path::new("/source"),
                41
            )
            .unwrap()
        );
    }

    #[test]
    fn t67_9_mount_roots_detect_an_alias_of_a_volume_inside_sources() {
        let info = b"41 0 0:35 /@ / rw - btrfs /dev/root rw\n60 41 0:60 / /source/my\\040volume rw - ext4 /dev/other rw\n61 41 0:60 /folder /storage rw - ext4 /dev/other rw\n";
        assert!(
            mounted_paths_overlap(
                info,
                Path::new("/storage/stage"),
                61,
                Path::new("/source"),
                41
            )
            .unwrap()
        );
    }
}

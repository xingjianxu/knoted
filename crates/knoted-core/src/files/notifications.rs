//! Kernel notifications are hints: every event (including overflow) requests a
//! complete scan. No absence decision is made from an event stream.
use super::{
    safe_fs::{self, Identity},
    scanner::ScanRules,
};
use rustix::fs::{self, Dir, FileType, inotify};
use std::{
    collections::HashSet,
    mem::MaybeUninit,
    os::fd::{AsRawFd, OwnedFd},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

pub struct NotificationWatch {
    fd: OwnedFd,
    watches: HashSet<i32>,
}
impl NotificationWatch {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self {
            fd: inotify::init(inotify::CreateFlags::NONBLOCK | inotify::CreateFlags::CLOEXEC)?,
            watches: HashSet::new(),
        })
    }
    fn add(&mut self, directory: &OwnedFd) -> std::io::Result<()> {
        // /proc/self/fd resolves the already checked directory descriptor, not
        // a mutable user path. Intentionally follow this kernel-owned fd link.
        let path = format!("/proc/self/fd/{}", directory.as_raw_fd());
        let flags = inotify::WatchFlags::MODIFY
            | inotify::WatchFlags::ATTRIB
            | inotify::WatchFlags::CLOSE_WRITE
            | inotify::WatchFlags::CREATE
            | inotify::WatchFlags::DELETE
            | inotify::WatchFlags::MOVED_FROM
            | inotify::WatchFlags::MOVED_TO
            | inotify::WatchFlags::DELETE_SELF
            | inotify::WatchFlags::MOVE_SELF
            | inotify::WatchFlags::ONLYDIR;
        self.watches
            .insert(inotify::add_watch(&self.fd, path.as_str(), flags)?);
        Ok(())
    }
    /// Keep one persistent inotify descriptor: replacing it after a scan would
    /// drop events that arrived during that scan. Watch only registered roots;
    /// a safely opened existing ancestor can signal an absent root's return.
    pub fn refresh(
        &mut self,
        path: &Path,
        rules: &ScanRules,
        registered: Option<&Identity>,
    ) -> std::io::Result<()> {
        let previous = std::mem::take(&mut self.watches);
        let result = self.refresh_tree(path, rules, registered);
        if result.is_ok() {
            for wd in previous.difference(&self.watches) {
                let _ = inotify::remove_watch(&self.fd, *wd);
            }
        } else {
            self.watches.extend(previous);
        }
        result
    }
    fn refresh_tree(
        &mut self,
        path: &Path,
        rules: &ScanRules,
        registered: Option<&Identity>,
    ) -> std::io::Result<()> {
        let mut ancestor = path.parent();
        while let Some(parent) = ancestor {
            if let Ok(fd) = safe_fs::open_root(parent) {
                self.add(&fd)?;
                break;
            }
            ancestor = parent.parent();
        }
        let Some(registered) = registered else {
            return Ok(());
        };
        let Ok(root) = safe_fs::open_root(path) else {
            return Ok(());
        };
        if &safe_fs::metadata(&root)?.identity != registered {
            return Ok(());
        }
        let mut pending = vec![PathBuf::new()];
        while let Some(relative) = pending.pop() {
            let fd = safe_fs::open_directory(&root, &relative)?;
            self.add(&fd)?;
            for entry in Dir::read_from(&fd)? {
                let entry = entry?;
                let name = entry.file_name();
                if [b".".as_slice(), b"..".as_slice()].contains(&name.to_bytes()) {
                    continue;
                }
                let name = name.to_str().map_err(|_| std::io::ErrorKind::InvalidData)?;
                let path = relative.join(name);
                if rules.excluded(&path, true) {
                    continue;
                }
                let child = safe_fs::open_entry(&fd, entry.file_name())?;
                if FileType::from_raw_mode(fs::fstat(child)?.st_mode) == FileType::Directory {
                    pending.push(path);
                }
            }
        }
        Ok(())
    }
    pub fn changed(&mut self) -> std::io::Result<bool> {
        let mut buffer = [MaybeUninit::uninit(); 8192];
        let mut reader = inotify::Reader::new(&self.fd, &mut buffer);
        let mut changed = false;
        // Bounded draining prevents a hot producer from starving the independent
        // rescan clock. Unread events remain in the same descriptor for next tick.
        for _ in 0..4096 {
            match reader.next() {
                Ok(event) => {
                    changed = true; // Q_OVERFLOW/IGNORED also require full reconciliation.
                    if event.events().contains(inotify::ReadFlags::IGNORED) {
                        self.watches.remove(&event.wd());
                    }
                }
                Err(rustix::io::Errno::WOULDBLOCK) => break,
                Err(error) => return Err(error.into()),
            }
        }
        Ok(changed)
    }
}
/// Debounce and supplemental deadlines are independent; a failed/busy enqueue
/// must not acknowledge pending changes. Events during an active job survive.
pub struct ScanSchedule {
    dirty: Option<Instant>,
    next_rescan: Instant,
    debounce: Duration,
    rescan: Duration,
}
impl ScanSchedule {
    pub fn new(now: Instant, debounce: Duration, rescan: Duration) -> Self {
        Self {
            dirty: None,
            next_rescan: now,
            debounce,
            rescan,
        }
    }
    pub fn notify(&mut self, now: Instant) {
        self.dirty = Some(now);
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next_rescan
            || self
                .dirty
                .is_some_and(|time| now.duration_since(time) >= self.debounce)
    }
    pub fn enqueued(&mut self, now: Instant) {
        self.dirty = None;
        if now >= self.next_rescan {
            self.next_rescan = now + self.rescan;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t43_real_notifications_include_nested_edits() {
        let tmp = tempfile::tempdir_in("/data/knoted-dev/tasks/t43/tmp").unwrap();
        let source = tmp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::create_dir(source.join("nested")).unwrap();
        let mut watch = NotificationWatch::new().unwrap();
        let identity = safe_fs::metadata(safe_fs::open_root(&source).unwrap())
            .unwrap()
            .identity;
        watch
            .refresh(&source, &ScanRules::default(), Some(&identity))
            .unwrap();
        assert!(!watch.changed().unwrap());
        std::fs::write(source.join("nested/a.txt"), "event").unwrap();
        assert!(
            watch.changed().unwrap(),
            "actual kernel notification must be consumed"
        );
        assert!(!watch.changed().unwrap());
    }
    #[test]
    fn t43_notifications_during_scan_survive_and_continuous_events_cannot_starve_rescan() {
        let start = Instant::now();
        let mut schedule =
            ScanSchedule::new(start, Duration::from_secs(2), Duration::from_secs(600));
        assert!(schedule.due(start));
        schedule.enqueued(start);
        schedule.notify(start + Duration::from_secs(1));
        assert!(!schedule.due(start + Duration::from_secs(2)));
        assert!(schedule.due(start + Duration::from_secs(3)));
        // A running job rejects enqueue: don't acknowledge the pending event.
        assert!(schedule.due(start + Duration::from_secs(30)));
        schedule.enqueued(start + Duration::from_secs(30));
        for second in 598..=600 {
            schedule.notify(start + Duration::from_secs(second));
        }
        assert!(
            schedule.due(start + Duration::from_secs(600)),
            "fixed supplemental deadline is independent of debounce"
        );
    }
}

#[cfg(test)]
mod overflow_tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn t43_overflow_and_events_pending_across_refresh_request_a_complete_scan() {
        // Exercise the actual inotify decoder with the kernel's overflow frame,
        // without a queue-flood performance test or changing host sysctls.
        let (read, mut write) = std::os::unix::net::UnixStream::pair().unwrap();
        read.set_nonblocking(true).unwrap();
        let mut watch = NotificationWatch {
            fd: read.into(),
            watches: HashSet::new(),
        };
        let mut event = Vec::new();
        event.extend_from_slice(&(-1_i32).to_ne_bytes());
        event.extend_from_slice(&libc::IN_Q_OVERFLOW.to_ne_bytes());
        event.extend_from_slice(&0_u32.to_ne_bytes());
        event.extend_from_slice(&0_u32.to_ne_bytes());
        write.write_all(&event).unwrap();
        assert!(watch.changed().unwrap());
        assert!(!watch.changed().unwrap());
    }
    #[test]
    fn t43_refresh_keeps_events_that_arrived_during_scan() {
        let tmp = tempfile::tempdir_in("/data/knoted-dev/tasks/t43/tmp").unwrap();
        let source = tmp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let identity = safe_fs::metadata(safe_fs::open_root(&source).unwrap())
            .unwrap()
            .identity;
        let mut watch = NotificationWatch::new().unwrap();
        watch
            .refresh(&source, &ScanRules::default(), Some(&identity))
            .unwrap();
        std::fs::write(source.join("during.txt"), "notification").unwrap();
        watch
            .refresh(&source, &ScanRules::default(), Some(&identity))
            .unwrap();
        assert!(
            watch.changed().unwrap(),
            "watch refresh must not replace descriptor and discard queued events"
        );
    }
}

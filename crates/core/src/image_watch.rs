//! SS-047 watcher planning, independent of the OS notification backend. Events
//! only nominate paths; a path is acted on after it has been quiet for
//! `DEBOUNCE` and two stats `STABILITY` apart agree. Too many paths, or a
//! backend that lost events, collapse into one complete refresh.
use crate::image_sources::WatchBatch;
use std::{
    collections::BTreeMap,
    fs,
    path::Path,
    time::{Duration, Instant},
};

pub const DEBOUNCE: Duration = Duration::from_millis(750);
pub const STABILITY: Duration = Duration::from_secs(1);
/// Distinct pending paths per source before falling back to a full refresh.
pub const MAX_PENDING: usize = 4096;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    File { size: u64, modified: Option<Duration> },
    Directory,
    Absent,
    /// Symlinks and special files: discovery never follows them either.
    Ignored,
}
/// Lstat one relative member without following a final symlink.
pub fn observe(root: &Path, relative: &str) -> Observation {
    match fs::symlink_metadata(root.join(relative)) {
        Ok(m) if m.is_file() => Observation::File {
            size: m.len(),
            modified: m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()),
        },
        Ok(m) if m.is_dir() => Observation::Directory,
        Ok(_) => Observation::Ignored,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Observation::Absent,
        // Access errors are left to the job, which reports them per member.
        Err(_) => Observation::File {
            size: 0,
            modified: None,
        },
    }
}

struct Pending {
    last_event: Instant,
    seen: Option<(Observation, Instant)>,
}
#[derive(Default)]
pub struct WatchPlanner {
    pending: BTreeMap<String, Pending>,
    /// Last event time once events were lost or exceeded the bound.
    overflow: Option<Instant>,
}
impl WatchPlanner {
    pub fn changed(&mut self, relative: String, now: Instant) {
        if let Some(last) = &mut self.overflow {
            *last = now;
            return;
        }
        if !self.pending.contains_key(&relative) && self.pending.len() >= MAX_PENDING {
            self.rescan(now);
            return;
        }
        let entry = self.pending.entry(relative).or_insert(Pending {
            last_event: now,
            seen: None,
        });
        entry.last_event = now;
        entry.seen = None;
    }
    /// The backend dropped or could not describe events.
    pub fn rescan(&mut self, now: Instant) {
        self.pending.clear();
        self.overflow = Some(now);
    }
    pub fn pending(&self) -> usize {
        self.pending.len()
    }
    pub fn overflowed(&self) -> bool {
        self.overflow.is_some()
    }
    /// Return at most one batch of settled changes. `observe` is only called
    /// for quiet paths, so a burst of saves costs one stat per settle attempt.
    pub fn poll(
        &mut self,
        now: Instant,
        mut observe: impl FnMut(&str) -> Observation,
    ) -> Option<WatchBatch> {
        if let Some(last) = self.overflow {
            if now.duration_since(last) < DEBOUNCE {
                return None;
            }
            self.overflow = None;
            return Some(WatchBatch::Refresh);
        }
        let mut settled = vec![];
        for (path, entry) in &mut self.pending {
            if now.duration_since(entry.last_event) < DEBOUNCE {
                continue;
            }
            let current = observe(path);
            match &entry.seen {
                Some((before, at)) if *before == current => {
                    if now.duration_since(*at) >= STABILITY {
                        settled.push((path.clone(), current));
                    }
                }
                _ => entry.seen = Some((current, now)),
            }
        }
        if settled.is_empty() {
            return None;
        }
        let (mut files, mut folders, mut removed) = (vec![], vec![], vec![]);
        for (path, observation) in settled {
            self.pending.remove(&path);
            match observation {
                Observation::File { .. } => files.push(path),
                Observation::Directory => folders.push(path),
                Observation::Absent => removed.push(path),
                Observation::Ignored => {}
            }
        }
        if files.is_empty() && folders.is_empty() && removed.is_empty() {
            return None;
        }
        Some(WatchBatch::Changes {
            files,
            folders,
            removed,
        })
    }
    /// Put a batch that could not be queued (another job owns the source) back.
    pub fn requeue(&mut self, batch: WatchBatch, now: Instant) {
        match batch {
            WatchBatch::Refresh => self.rescan(now),
            WatchBatch::Changes {
                files,
                folders,
                removed,
            } => {
                for path in files.into_iter().chain(folders).chain(removed) {
                    self.changed(path, now);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(size: u64) -> Observation {
        Observation::File {
            size,
            modified: None,
        }
    }
    #[test]
    fn rapid_saves_settle_once_after_quiet_and_stable_stats() {
        let mut p = WatchPlanner::default();
        let t = Instant::now();
        for i in 0..20 {
            p.changed("a.png".into(), t + Duration::from_millis(i * 100));
        }
        let last = t + Duration::from_millis(1900);
        let mut size = 1;
        // Still being written: quiet, but the size keeps changing.
        assert_eq!(p.poll(last + DEBOUNCE, |_| file(size)), None);
        size = 2;
        assert_eq!(p.poll(last + DEBOUNCE + STABILITY, |_| file(size)), None);
        assert_eq!(p.poll(last + DEBOUNCE + STABILITY * 2 - Duration::from_millis(1), |_| file(size)), None);
        assert_eq!(
            p.poll(last + DEBOUNCE + STABILITY * 2, |_| file(size)),
            Some(WatchBatch::Changes {
                files: vec!["a.png".into()],
                folders: vec![],
                removed: vec![]
            })
        );
        assert_eq!(p.pending(), 0);
        assert_eq!(p.poll(last + Duration::from_secs(60), |_| file(size)), None);
    }
    #[test]
    fn new_event_restarts_debounce_and_stability() {
        let mut p = WatchPlanner::default();
        let t = Instant::now();
        p.changed("a.png".into(), t);
        assert_eq!(p.poll(t + DEBOUNCE, |_| file(1)), None);
        p.changed("a.png".into(), t + DEBOUNCE + Duration::from_millis(10));
        // Quiet again, but the earlier stat no longer counts: observe afresh.
        let restat = t + DEBOUNCE + STABILITY;
        assert_eq!(p.poll(restat, |_| file(1)), None);
        assert_eq!(p.poll(restat + STABILITY - Duration::from_millis(1), |_| file(1)), None);
        assert!(p.poll(restat + STABILITY, |_| file(1)).is_some());
    }
    #[test]
    fn temp_file_rename_reports_only_the_final_name_and_deletions() {
        let mut p = WatchPlanner::default();
        let t = Instant::now();
        // A filtered temp name never reaches the planner; the rename source is gone.
        p.changed("old.png".into(), t);
        p.changed("new.png".into(), t);
        p.changed("folder".into(), t);
        let probe = |path: &str| match path {
            "old.png" => Observation::Absent,
            "folder" => Observation::Directory,
            _ => file(9),
        };
        assert_eq!(p.poll(t + DEBOUNCE, probe), None);
        assert_eq!(
            p.poll(t + DEBOUNCE + STABILITY, probe),
            Some(WatchBatch::Changes {
                files: vec!["new.png".into()],
                folders: vec!["folder".into()],
                removed: vec!["old.png".into()]
            })
        );
    }
    #[test]
    fn overflow_is_bounded_and_becomes_one_debounced_refresh() {
        let mut p = WatchPlanner::default();
        let t = Instant::now();
        for i in 0..MAX_PENDING {
            p.changed(format!("{i}.png"), t);
        }
        assert_eq!(p.pending(), MAX_PENDING);
        p.changed("one-too-many.png".into(), t);
        assert!(p.overflowed());
        assert_eq!(p.pending(), 0);
        // Events during overflow only extend the quiet period; nothing is stored.
        p.changed("later.png".into(), t + DEBOUNCE);
        assert_eq!(p.poll(t + DEBOUNCE + Duration::from_millis(1), |_| unreachable!()), None);
        assert_eq!(p.poll(t + DEBOUNCE * 2, |_| unreachable!()), Some(WatchBatch::Refresh));
        assert!(!p.overflowed());
        assert_eq!(p.poll(t + DEBOUNCE * 4, |_| unreachable!()), None);
    }
    #[test]
    fn ignored_entries_are_dropped_and_requeue_preserves_work() {
        let mut p = WatchPlanner::default();
        let t = Instant::now();
        p.changed("link".into(), t);
        assert_eq!(p.poll(t + DEBOUNCE, |_| Observation::Ignored), None);
        assert_eq!(p.poll(t + DEBOUNCE + STABILITY, |_| Observation::Ignored), None);
        assert_eq!(p.pending(), 0);
        p.requeue(
            WatchBatch::Changes {
                files: vec!["a.png".into()],
                folders: vec![],
                removed: vec![],
            },
            t,
        );
        assert_eq!(p.pending(), 1);
        p.requeue(WatchBatch::Refresh, t);
        assert!(p.overflowed());
    }
    #[test]
    fn observe_distinguishes_files_folders_absence_and_links() {
        let d = tempfile::tempdir().unwrap();
        fs::write(d.path().join("a.png"), b"12").unwrap();
        fs::create_dir(d.path().join("dir")).unwrap();
        assert!(matches!(observe(d.path(), "a.png"), Observation::File { size: 2, .. }));
        assert_eq!(observe(d.path(), "dir"), Observation::Directory);
        assert_eq!(observe(d.path(), "gone.png"), Observation::Absent);
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(d.path().join("a.png"), d.path().join("link.png")).unwrap();
            assert_eq!(observe(d.path(), "link.png"), Observation::Ignored);
        }
    }
}

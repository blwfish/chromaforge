//! A failed save is an error, not a silent success: a command whose journal append fails returns
//! [`EngineError::NotSaved`], its change stays applied and queued, and the next save writes it.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use lcc_catalog::{Flag, MemStore, SnapshotPolicy, Store};
use serde_json::json;

use crate::library::LibraryStores;
use crate::{EngineError, Session};

/// A store whose writes fail while `fail` is set (a full or unplugged disk).
#[derive(Clone, Default)]
struct Flaky {
    files: MemStore,
    fail: Arc<AtomicBool>,
    /// Only snapshots fail (appends work).
    snapshots_only: bool,
    /// Only appends fail (a dead log handle; fresh files can still be written).
    appends_only: bool,
}

impl Flaky {
    fn failing(&self) -> bool {
        self.fail.load(Ordering::SeqCst)
    }
    fn set_failing(&self, on: bool) {
        self.fail.store(on, Ordering::SeqCst);
    }
    /// The files as a crash at this moment would leave them.
    fn crash_copy(&self) -> Flaky {
        let m = MemStore::new();
        *m.files.lock().unwrap() = self.files.files.lock().unwrap().clone();
        Flaky { files: m, ..Default::default() }
    }
}

impl Store for Flaky {
    fn read(&mut self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.files.read(name)
    }
    fn write_atomic(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if self.failing() && !self.appends_only {
            return Err(std::io::Error::other("disk full"));
        }
        self.files.write_atomic(name, data)
    }
    fn append(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if self.failing() && !self.snapshots_only {
            return Err(std::io::Error::other("disk full"));
        }
        self.files.append(name, data)
    }
    fn truncate(&mut self, name: &str, len: u64) -> std::io::Result<()> {
        self.files.truncate(name, len)
    }
    fn describe(&self) -> String {
        "flaky".into()
    }
}

fn open(store: &Flaky, seed: bool) -> Session {
    let mut s = Session::new();
    let stores = LibraryStores { dir: "flaky".into(), catalog: Box::new(store.clone()), files: Box::new(MemStore::new()), on_disk: false };
    s.open_library_in(stores, seed).unwrap();
    s
}

#[test]
fn failed_append_fails_the_command_and_is_retried() {
    let store = Flaky::default();
    let mut s = open(&store, true);
    let id = s.selection.active.unwrap();
    s.execute("photo.rate", &json!({"rating": 1})).unwrap();
    store.set_failing(true);

    let e = s.execute("photo.rate", &json!({"rating": 4})).unwrap_err();
    assert!(matches!(e, EngineError::NotSaved(_)), "{e:?}");
    let msg = e.to_string();
    assert!(
        msg.starts_with("saved in memory but not written to disk: ") && msg.contains("disk full") && msg.ends_with("the app will retry"),
        "{msg}"
    );
    // applied in memory, undoable, queued
    assert_eq!(s.catalog.photo(id).unwrap().rating, 4);
    assert!(!s.undo.is_empty());
    assert_eq!(s.unsaved().map(|u| u.0), Some(1));
    // queries and commands that change nothing are unaffected (they retry the queue, but report
    // their own result)
    let info = s.execute("library.info", &json!({})).unwrap();
    assert_eq!(info["unsavedOps"], 1, "{info}");
    assert!(info["unsavedError"].as_str().unwrap().contains("disk full"), "{info}");
    s.execute("library.select", &json!({"ids": [id.0]})).unwrap();
    // a second change fails too; both stay queued, in order
    let e = s.execute("photo.flag", &json!({"flag": "pick"})).unwrap_err();
    assert!(matches!(e, EngineError::NotSaved(_)));
    assert_eq!(s.unsaved().map(|u| u.0), Some(2));
    // the frame loop backs off instead of retrying every frame
    s.persist_if_dirty();
    assert_eq!(s.unsaved().map(|u| u.0), Some(2));
    // a crash now loses only the unsaved changes; the earlier ones are on disk
    assert_eq!(open(&store.crash_copy(), true).catalog.photo(id).unwrap().rating, 1);

    // the disk comes back: the next save writes the whole queue
    store.set_failing(false);
    s.execute("library.info", &json!({})).unwrap();
    assert_eq!(s.unsaved(), None);
    assert!(s.library.as_ref().unwrap().last_error.is_none());
    let expect = s.catalog.to_snapshot();
    drop(s); // no close: like a crash
    let s2 = open(&store, true);
    assert_eq!(s2.catalog.to_snapshot(), expect);
    let p = s2.catalog.photo(id).unwrap();
    assert_eq!((p.rating, p.flag), (4, Flag::Pick));
}

/// Undo is a command like any other: a failed write of its op is reported, and retried.
#[test]
fn failed_append_of_undo_is_reported_and_retried() {
    let store = Flaky::default();
    let mut s = open(&store, true);
    let id = s.selection.active.unwrap();
    let before = s.catalog.photo(id).unwrap().rating;
    s.execute("photo.rate", &json!({"rating": 3})).unwrap();
    store.set_failing(true);
    assert!(matches!(s.execute("edit.undo", &json!({})), Err(EngineError::NotSaved(_))));
    assert_eq!(s.catalog.photo(id).unwrap().rating, before);
    store.set_failing(false);
    s.persist().unwrap();
    drop(s);
    assert_eq!(open(&store, true).catalog.photo(id).unwrap().rating, before);
}

/// A failed compaction is not a failed command: the command's own records were appended.
#[test]
fn failed_snapshot_does_not_fail_the_command() {
    let store = Flaky { snapshots_only: true, ..Default::default() };
    let mut s = open(&store, false);
    s.library.as_mut().unwrap().journal_mut().policy = SnapshotPolicy { max_records: 1, max_bytes: u64::MAX };
    store.set_failing(true);
    s.execute("album.create", &json!({"name": "A"})).unwrap();
    s.execute("album.create", &json!({"name": "B"})).unwrap();
    assert_eq!(s.unsaved(), None);
    assert!(s.library.as_ref().unwrap().last_error.is_some(), "the compaction failure is reported");
    drop(s);
    store.set_failing(false);
    assert_eq!(open(&store, false).catalog.albums().count(), 2);
}

/// Issue #101: quitting while the log can't be appended to still saves the queued changes in the
/// closing snapshot (a fresh file), and reports success; they are not appended again later.
#[test]
fn close_saves_queued_changes_in_the_snapshot_when_appends_fail() {
    let store = Flaky { appends_only: true, ..Default::default() };
    let mut s = open(&store, true);
    let id = s.selection.active.unwrap();
    store.set_failing(true);
    assert!(matches!(s.execute("photo.rate", &json!({"rating": 4})), Err(EngineError::NotSaved(_))));
    assert!(matches!(s.execute("photo.flag", &json!({"flag": "pick"})), Err(EngineError::NotSaved(_))));
    assert_eq!(s.unsaved().map(|u| u.0), Some(2));
    s.close_library().unwrap();
    assert_eq!(s.unsaved(), None);
    assert!(s.pending_log.is_empty());
    let expect = s.catalog.to_snapshot();
    drop(s);
    store.set_failing(false);
    let mut s2 = open(&store, true);
    assert_eq!(s2.catalog.to_snapshot(), expect);
    let p = s2.catalog.photo(id).unwrap();
    assert_eq!((p.rating, p.flag), (4, Flag::Pick));
    // later edits continue after the absorbed ops
    s2.execute("photo.rate", &json!({"rating": 2})).unwrap();
    let expect = s2.catalog.to_snapshot();
    drop(s2);
    assert_eq!(open(&store, true).catalog.to_snapshot(), expect);
}

/// When neither the log nor the snapshot can be written, closing fails with `NotSaved` and the
/// changes stay queued (a later retry can still save them).
#[test]
fn close_fails_when_nothing_can_be_saved() {
    let store = Flaky::default();
    let mut s = open(&store, true);
    store.set_failing(true);
    assert!(s.execute("photo.rate", &json!({"rating": 4})).is_err());
    let e = s.close_library().unwrap_err();
    assert!(matches!(e, EngineError::NotSaved(_)), "{e:?}");
    assert_eq!(s.unsaved().map(|u| u.0), Some(1));
    store.set_failing(false);
    s.close_library().unwrap();
    assert_eq!(s.unsaved(), None);
}

/// Issue #2266: a presets.json that can't be written isn't only logged: the preset counts as
/// unsaved (the top bar warns, the web backup refuses) until a later save writes it.
#[test]
fn a_preset_that_cannot_be_written_counts_as_unsaved_until_it_is() {
    let files = Flaky::default();
    let mut s = Session::new();
    let stores = LibraryStores { dir: "flaky".into(), catalog: Box::new(MemStore::new()), files: Box::new(files.clone()), on_disk: false };
    s.open_library_in(stores, true).unwrap();
    assert!(s.unsaved().is_none());
    files.set_failing(true);
    s.execute("preset.create", &json!({"name": "Mine"})).unwrap();
    let (n, why) = s.unsaved().expect("the preset is unsaved");
    assert_eq!(n, 1);
    assert!(why.contains("presets") && why.contains("disk full"), "{why}");
    let stored = |f: &Flaky| f.files.files.lock().unwrap().get("presets.json").cloned().unwrap_or_default();
    assert!(!String::from_utf8_lossy(&stored(&files)).contains("Mine"));
    // still failing: still unsaved
    s.persist().unwrap();
    assert!(s.unsaved().is_some());
    files.set_failing(false);
    s.persist().unwrap();
    assert!(s.unsaved().is_none(), "written once the disk works again");
    assert!(String::from_utf8_lossy(&stored(&files)).contains("Mine"));
}

/// A store whose log is written "by a thread of its own", as natively: an append is handed over at once, and a
/// failed write shows up later in [`Store::log_progress`]. Its log write and the log reset after a snapshot can be
/// made to fail.
#[derive(Clone, Default)]
struct ThreadedLog {
    files: MemStore,
    /// The next log writes fail (a full disk); what was handed over is left out.
    log_fails: Arc<AtomicBool>,
    /// Replacing the log (its reset after a snapshot) fails.
    reset_fails: Arc<AtomicBool>,
    /// (durable length of the log, why a write failed)
    progress: Arc<std::sync::Mutex<(u64, Option<String>)>>,
    /// A failed write the thread hasn't reported yet.
    unreported: Arc<std::sync::Mutex<Option<String>>>,
}

impl ThreadedLog {
    /// The thread gets round to reporting its failed write.
    fn report_failure(&self) {
        let why = self.unreported.lock().unwrap().take();
        self.progress.lock().unwrap().1 = why;
    }
}

impl Store for ThreadedLog {
    fn read(&mut self, name: &str) -> std::io::Result<Option<Vec<u8>>> {
        self.files.read(name)
    }
    fn write_atomic(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if name == lcc_catalog::journal::LOG {
            if self.reset_fails.load(Ordering::SeqCst) {
                return Err(std::io::Error::other("share unavailable"));
            }
            *self.progress.lock().unwrap() = (data.len() as u64, None);
        }
        self.files.write_atomic(name, data)
    }
    fn append(&mut self, name: &str, data: &[u8]) -> std::io::Result<()> {
        if name != lcc_catalog::journal::LOG {
            return self.files.append(name, data);
        }
        let mut p = self.progress.lock().unwrap();
        let mut unreported = self.unreported.lock().unwrap();
        if p.1.is_some() || unreported.is_some() {
            return Ok(()); // left out until the log is cut back or replaced, as the thread does
        }
        if self.log_fails.load(Ordering::SeqCst) {
            *unreported = Some("no space left on the device".into());
            return Ok(());
        }
        self.files.append(name, data)?;
        p.0 += data.len() as u64;
        Ok(())
    }
    fn truncate(&mut self, name: &str, len: u64) -> std::io::Result<()> {
        self.files.truncate(name, len)?;
        if name == lcc_catalog::journal::LOG {
            let mut p = self.progress.lock().unwrap();
            *p = (p.0.min(len), None);
        }
        Ok(())
    }
    fn describe(&self) -> String {
        "threaded log".into()
    }
    fn log_progress(&self, name: &str) -> Option<lcc_catalog::store::LogProgress> {
        (name == lcc_catalog::journal::LOG).then(|| {
            let p = self.progress.lock().unwrap();
            lcc_catalog::store::LogProgress { durable_len: p.0, failed: p.1.clone() }
        })
    }
}

fn open_threaded(store: &ThreadedLog, seed: bool) -> (Session, lcc_catalog::LoadReport) {
    let mut s = Session::new();
    let stores = LibraryStores { dir: "threaded".into(), catalog: Box::new(store.clone()), files: Box::new(MemStore::new()), on_disk: false };
    let report = s.open_library_in(stores, seed).unwrap().clone();
    (s, report)
}

/// A log write fails, the snapshot that saves what it lost is durable, then resetting the log after it fails (a
/// stalled share). The snapshot holds the queued change: the command succeeds and the change is never appended
/// again. (It used to be appended under a new number, and the next open replayed it a second time: a duplicate
/// History step, or a duplicate added photo counted only as a failure.)
#[test]
fn a_change_a_durable_snapshot_saved_is_not_appended_again_when_the_log_reset_fails() {
    let store = ThreadedLog::default();
    let (mut s, _) = open_threaded(&store, true);
    let id = s.selection.active.unwrap();
    s.execute("photo.rate", &json!({"rating": 1})).unwrap();
    store.log_fails.store(true, Ordering::SeqCst);
    s.execute("photo.rate", &json!({"rating": 4})).unwrap(); // handed over; its write fails afterwards
    store.log_fails.store(false, Ordering::SeqCst);
    store.report_failure();
    store.reset_fails.store(true, Ordering::SeqCst);
    // the next append finds the lost one and is refused; a snapshot saves both, then the log reset fails
    let journal = |s: &Session| (s.library.as_ref().unwrap().journal().seq(), s.library.as_ref().unwrap().journal().lost());
    let (seq, lost) = journal(&s);
    let flagged = s.execute("photo.flag", &json!({"flag": "pick"}));
    assert_eq!(journal(&s), (seq + lost + 1, 0), "the snapshot holds the lost and the queued change");
    assert!(flagged.is_ok(), "the change is saved: {flagged:?}");
    assert!(s.pending_log.is_empty(), "so it is not queued for the log any more");
    assert_eq!(s.unsaved(), None);
    store.reset_fails.store(false, Ordering::SeqCst);
    s.execute("photo.rate", &json!({"rating": 2})).unwrap();
    s.flush_catalog().unwrap();
    let expect = s.catalog.to_snapshot();
    drop(s);
    let (s2, r) = open_threaded(&store, true);
    assert_eq!(s2.catalog.to_snapshot(), expect);
    let p = s2.catalog.photo(id).unwrap();
    assert_eq!((p.rating, p.flag), (2, Flag::Pick));
    assert_eq!((r.replayed, r.failed), (1, 0), "only the change after the snapshot replays: {r:?}");
}

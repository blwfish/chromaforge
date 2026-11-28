//! The built-in Hard Drive service: each published collection is a folder inside the service's
//! folder, kept in sync with the collection (new and modified photos are written, removed ones
//! deleted). Remote ids are file names inside the collection's folder.
//!
//! Only this collection's own files are ever removed: the published file (its remote id) and the
//! sidecars recorded as written with it ([`CollectionConfig::sidecars`]), never a file that merely
//! shares the stem (a catalogued original's `.xmp` in the same folder, #1635). Removed files go to
//! the system trash when the session has one, and every write and removal is first checked
//! against the library's originals ([`HardDrive::with_guard`]): a file that is an original's, or
//! has become one, is never replaced and is left where it is.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::config::CollectionConfig;
use crate::service::{PublishError, PublishService, Published, Upload};
use crate::{KIND_HARD_DRIVE, ServiceConfig};

/// A name usable as one path segment: separators and control characters become `_`; empty, `.`
/// and `..` are refused.
pub fn safe_name(s: &str) -> Option<String> {
    let n: String = s.trim().chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') || c.is_control() { '_' } else { c }).collect();
    let n = n.trim().to_string();
    if n.is_empty() || n == "." || n == ".." { None } else { Some(n) }
}

/// The service's folder (`settings.dir`).
pub fn service_dir(svc: &ServiceConfig) -> Result<PathBuf, PublishError> {
    let dir = svc.settings.get("dir").and_then(|d| d.as_str()).map(str::trim).filter(|d| !d.is_empty());
    dir.map(PathBuf::from).ok_or_else(|| PublishError::Config(format!("the publish service `{}` has no folder: set one", svc.name)))
}

/// Checks a path before it is written or removed: `Err` (a message for people) when it is a
/// catalogued original or one of its sidecars.
pub type GuardFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// Moves one file to the system trash (never deletes it outright). `Err` carries a message for people.
pub type TrashFn = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

pub struct HardDrive {
    folder: PathBuf,
    /// Per published file, the sidecars this collection recorded writing with it.
    recorded: BTreeMap<String, Vec<String>>,
    guard: Option<GuardFn>,
    trash: Option<TrashFn>,
}

impl HardDrive {
    pub fn open(svc: &ServiceConfig, coll: &CollectionConfig) -> Result<HardDrive, PublishError> {
        let folder = safe_name(&coll.folder).ok_or_else(|| PublishError::Config(format!("`{}` can't be a folder name", coll.folder)))?;
        Ok(HardDrive { folder: service_dir(svc)?.join(folder), recorded: coll.sidecars.clone(), guard: None, trash: None })
    }

    /// Check every file before it is written or removed (the library's originals guard).
    pub fn with_guard(mut self, guard: GuardFn) -> Self {
        self.guard = Some(guard);
        self
    }

    /// Move removed files to the trash instead of deleting them (none on the web and Android:
    /// there a removed file, always one this collection wrote, is deleted).
    pub fn with_trash(mut self, trash: TrashFn) -> Self {
        self.trash = Some(trash);
        self
    }

    /// The sidecars recorded as written with the published file `name`.
    fn recorded_sidecars(&self, name: &str) -> &[String] {
        self.recorded.get(name).map_or(&[], Vec::as_slice)
    }

    /// `Err` when writing `path` would replace (or create) an original's file or sidecar.
    fn check(&self, path: &Path) -> Result<(), PublishError> {
        match &self.guard {
            Some(guard) => guard(path).map_err(PublishError::Io),
            None => Ok(()),
        }
    }

    /// The collection's folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    fn path_of(&self, remote_id: &str) -> Result<PathBuf, PublishError> {
        // remote ids come from the catalog: never let one point outside the folder
        match safe_name(remote_id) {
            Some(n) if n == remote_id => Ok(self.folder.join(n)),
            _ => Err(PublishError::Config(format!("`{remote_id}` is not a file name in the collection's folder"))),
        }
    }

    /// `wanted`, or the first of `a-2.jpg`, `a-3.jpg`, ... (the app's naming for clashes) whose file
    /// and sidecars don't belong to another photo. Files of this photo's previous publication
    /// don't count: a re-publish replaces them.
    ///
    /// A file of the wanted name holding exactly these bytes is taken as this photo's too: a run stopped before it
    /// recorded what it sent (a crash, a power cut, the app closed) left it, and the next run would otherwise send the
    /// photo again as `a-2.jpg` beside it (issue #1376). Its sidecars go with it.
    fn free_name(&self, wanted: &str, up: &Upload<'_>) -> String {
        if up.previous.is_none() && self.holds(wanted, up.bytes) {
            return wanted.to_string();
        }
        // the previous publication's file and the sidecars recorded with it. One with no record (published before
        // sidecars were recorded) keeps the old rule, its sidecar named like it: replacing it is checked against the
        // originals guard in `publish`, and records it from then on. Removal never uses that rule.
        let own = |n: &str| {
            up.previous.is_some_and(|p| {
                let recorded = self.recorded_sidecars(p);
                n == p || recorded.iter().any(|r| r == n) || (recorded.is_empty() && up.sidecars.iter().any(|(ext, _)| with_ext(p, ext) == n))
            })
        };
        let taken = |n: &str| !own(n) && self.folder.join(n).exists();
        let free = |n: &str| !taken(n) && !up.sidecars.iter().any(|(ext, _)| taken(&with_ext(n, ext)));
        if free(wanted) {
            return wanted.to_string();
        }
        let (stem, ext) = match wanted.rsplit_once('.') {
            Some((s, e)) if !s.is_empty() => (s, Some(e)),
            _ => (wanted, None),
        };
        (2u32..)
            .map(|i| match ext {
                Some(e) => format!("{stem}-{i}.{e}"),
                None => format!("{stem}-{i}"),
            })
            .find(|n| free(n))
            .unwrap_or_else(|| wanted.to_string())
    }
}

impl HardDrive {
    /// Whether the collection's file `name` holds exactly `bytes` (compared only when the sizes agree).
    fn holds(&self, name: &str, bytes: &[u8]) -> bool {
        let path = self.folder.join(name);
        std::fs::metadata(&path).is_ok_and(|m| m.is_file() && m.len() == bytes.len() as u64) && std::fs::read(&path).is_ok_and(|b| b == bytes)
    }
}

/// Whether `sidecar` is named like a sidecar of `name` (same stem, another extension). A record
/// read from `publish.json` that names anything else is ignored.
fn is_sidecar_of(name: &str, sidecar: &str) -> bool {
    let stem = |n: &str| n.rsplit_once('.').map_or(n, |(s, _)| s).to_string();
    sidecar != name && sidecar.contains('.') && stem(sidecar) == stem(name)
}

fn remove_file(path: &Path) -> Result<(), PublishError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(PublishError::Io(format!("{}: {e}", path.display()))),
    }
}

/// `name` with its extension replaced by `ext` (a sidecar's name).
pub fn with_ext(name: &str, ext: &str) -> String {
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name);
    format!("{stem}.{ext}")
}

impl PublishService for HardDrive {
    fn kind(&self) -> &'static str {
        KIND_HARD_DRIVE
    }

    fn publish(&mut self, up: &Upload<'_>) -> Result<Published, PublishError> {
        let wanted = safe_name(up.file_name).ok_or_else(|| PublishError::Config(format!("`{}` can't be a file name", up.file_name)))?;
        std::fs::create_dir_all(&self.folder).map_err(|e| PublishError::Io(format!("{}: {e}", self.folder.display())))?;
        let name = self.free_name(&wanted, up);
        let path = self.folder.join(&name);
        let sidecars: Vec<String> = up.sidecars.iter().map(|(ext, _)| with_ext(&name, ext)).collect();
        // never over an original or its sidecar, whatever name the folder left free
        self.check(&path)?;
        for sc in &sidecars {
            self.check(&self.folder.join(sc))?;
        }
        lcc_catalog::safe_file::write_atomic(&path, up.bytes).map_err(|e| PublishError::Io(format!("{}: {e}", path.display())))?;
        for ((_, bytes), sidecar) in up.sidecars.iter().zip(&sidecars) {
            let p = self.folder.join(sidecar);
            lcc_catalog::safe_file::write_atomic(&p, bytes).map_err(|e| PublishError::Io(format!("{}: {e}", p.display())))?;
        }
        let mut written = vec![name.clone()];
        written.extend(sidecars.iter().cloned());
        // a re-publish under another name (the naming template changed): the old file goes
        if let Some(prev) = up.previous.filter(|p| *p != name) {
            self.remove_keeping(prev, &written)?;
        }
        if sidecars.is_empty() {
            self.recorded.remove(&name);
        } else {
            self.recorded.insert(name.clone(), sidecars.clone());
        }
        Ok(Published { remote_id: name, sidecars })
    }

    fn remove(&mut self, remote_id: &str) -> Result<(), PublishError> {
        self.remove_keeping(remote_id, &[])
    }
}

impl HardDrive {
    /// Remove `remote_id` and the sidecars recorded as written with it, except files named in
    /// `keep` (just written by the publish that replaces it: `a.dng` -> `a.tif` shares the sidecar
    /// `a.xmp`). A file that shares the stem but wasn't recorded (an original's own `.xmp`) stays.
    fn remove_keeping(&mut self, remote_id: &str, keep: &[String]) -> Result<(), PublishError> {
        let path = self.path_of(remote_id)?;
        let sidecars: Vec<PathBuf> = self
            .recorded_sidecars(remote_id)
            .iter()
            .filter(|sc| !keep.contains(sc) && is_sidecar_of(remote_id, sc))
            .filter_map(|sc| self.path_of(sc).ok())
            .collect();
        for sc in &sidecars {
            self.discard(sc)?;
        }
        self.discard(&path)?;
        self.recorded.remove(remote_id);
        Ok(())
    }

    /// Take one of this collection's files away: to the trash when there is one. Already gone is
    /// fine; a folder, or a file that is (or has become) a catalogued original's, is left where it is.
    fn discard(&self, path: &Path) -> Result<(), PublishError> {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Ok(m) if m.is_dir() => return Ok(()),
            _ => {}
        }
        if self.check(path).is_err() {
            return Ok(()); // not this collection's to remove any more
        }
        match &self.trash {
            Some(trash) => trash(path).map_err(|e| PublishError::Io(format!("{}: can't move it to the trash: {e}", path.display()))),
            None => remove_file(path),
        }
    }
}

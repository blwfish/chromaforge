use lcc_catalog::{Album, AlbumId, Catalog, Op, Photo, PhotoId, RemoteIdentity, Source, SyncState};
use serde_json::json;

use crate::hard_drive::{HardDrive, safe_name};
use crate::*;

/// An empty Temp folder of the test's own, removed when the test ends.
fn tmp(tag: &str) -> lcc_testkit::TempDir {
    static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    lcc_testkit::TempDir::new(&format!("lcc-publish-{tag}-{n}"))
}

fn catalog(n: u64) -> (Catalog, AlbumId) {
    let mut c = Catalog::new();
    for _ in 0..n {
        let id = c.alloc_photo_id();
        let p = Photo::new(id, Source::File { path: format!("/x/{}.jpg", id.0) }, &format!("{}.jpg", id.0), "jpeg", 10, 10, "2026-01-01T00:00:00");
        c.apply(Op::AddPhoto { photo: Box::new(p) }).unwrap();
    }
    let a = c.alloc_album_id();
    let mut album = Album::new(a, "Web");
    album.photos = (1..=n).map(PhotoId).collect();
    c.apply(Op::AddAlbum { album }).unwrap();
    (c, a)
}

fn link(c: &mut Catalog, a: AlbumId, id: u64, fp: String) {
    let r = RemoteIdentity {
        photo_id: PhotoId(id),
        service: SERVICE.into(),
        account_id: account_id("svc-1", a),
        remote_id: format!("{id}.jpg"),
        remote_checksum: Some(fp),
        remote_updated_at: None,
        last_synced_at: None,
        sync_state: SyncState::Synced,
    };
    c.apply(Catalog::link_remote_op(r)).unwrap();
}

#[test]
fn states_follow_links_and_edits() {
    let (mut c, a) = catalog(4);
    let fp = |c: &Catalog, i| fingerprint(c.photo(PhotoId(i)).unwrap());
    let f2 = fp(&c, 2);
    link(&mut c, a, 2, f2);
    link(&mut c, a, 3, "stale".into());
    let f4 = fp(&c, 4);
    link(&mut c, a, 4, f4);
    // 4 leaves the collection
    c.apply(Op::SetAlbumPhotos { id: a, photos: vec![PhotoId(1), PhotoId(2), PhotoId(3)] }).unwrap();
    let st = status(&c, "svc-1", a);
    assert_eq!(st.new, vec![PhotoId(1)]);
    assert_eq!(st.published, vec![PhotoId(2)]);
    assert_eq!(st.modified, vec![PhotoId(3)]);
    assert_eq!(st.to_remove, vec![PhotoId(4)]);
    assert_eq!(st.pending(), 3);
    assert_eq!(st.state_of(PhotoId(3)), Some(PhotoState::Modified));
    // a rating change makes a published photo modified
    c.apply(Op::SetRating { id: PhotoId(2), rating: 5 }).unwrap();
    assert_eq!(status(&c, "svc-1", a).modified, vec![PhotoId(2), PhotoId(3)]);
}

#[test]
fn a_missing_album_only_removes() {
    let (mut c, a) = catalog(1);
    link(&mut c, a, 1, "x".into());
    let st = status(&c, "svc-1", AlbumId(999));
    assert_eq!(st.pending(), 0);
    c.apply(Op::RemoveAlbum { id: a }).unwrap();
    assert_eq!(status(&c, "svc-1", a).to_remove, vec![PhotoId(1)]);
}

#[test]
fn config_round_trips_and_damage_is_an_error() {
    let d = tmp("cfg");
    assert_eq!(PublishConfig::load(&d).unwrap(), PublishConfig::default());
    let mut c = PublishConfig::default();
    let id = c.alloc_id();
    c.services.push(ServiceConfig {
        id: id.clone(),
        kind: KIND_HARD_DRIVE.into(),
        name: "Disk".into(),
        settings: json!({"dir": "/tmp"}),
        export: json!({"format": "jpeg"}),
        set: Some(AlbumId(1)),
        collections: vec![CollectionConfig { album: AlbumId(2), folder: "Web".into(), sidecars: Default::default() }],
    });
    c.save(&d).unwrap();
    let back = PublishConfig::load(&d).unwrap();
    assert_eq!(back, c);
    assert_eq!(back.service_of(AlbumId(2)).map(|s| s.id.as_str()), Some(id.as_str()));
    let mut back = back;
    assert_ne!(back.alloc_id(), id);
    std::fs::write(d.join(FILE), b"{not json").unwrap();
    assert!(matches!(PublishConfig::load(&d), Err(PublishError::Config(_))));
}

fn svc(dir: &std::path::Path) -> (ServiceConfig, CollectionConfig) {
    let coll = CollectionConfig { album: AlbumId(2), folder: "Web".into(), sidecars: Default::default() };
    let s = ServiceConfig {
        id: "svc-1".into(),
        kind: KIND_HARD_DRIVE.into(),
        name: "Disk".into(),
        settings: json!({"dir": dir.to_string_lossy()}),
        export: json!({}),
        set: None,
        collections: vec![coll.clone()],
    };
    (s, coll)
}

#[test]
fn hard_drive_writes_replaces_and_removes() {
    let d = tmp("hd");
    let (s, coll) = svc(&d);
    let mut hd = open_service(&s, &coll).unwrap();
    let side = [("xmp", b"<x/>".to_vec())];
    let up = Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one", sidecars: &side, previous: None };
    let r = hd.publish(&up).unwrap();
    assert_eq!(r.remote_id, "a.jpg");
    assert_eq!(std::fs::read(d.join("Web/a.jpg")).unwrap(), b"one");
    assert!(d.join("Web/a.xmp").exists());
    // re-published under a new name: the old file goes
    let up = Upload { photo: PhotoId(1), file_name: "b.jpg", bytes: b"two", sidecars: &[], previous: Some("a.jpg") };
    assert_eq!(hd.publish(&up).unwrap().remote_id, "b.jpg");
    assert!(!d.join("Web/a.jpg").exists() && !d.join("Web/a.xmp").exists());
    hd.remove("b.jpg").unwrap();
    assert!(!d.join("Web/b.jpg").exists());
    // already gone is fine
    hd.remove("b.jpg").unwrap();
}

/// #1635: a removal takes only the files the collection recorded writing, through the trash, and never one the
/// originals guard protects; a write the guard refuses fails and writes nothing.
#[test]
fn hard_drive_removes_only_its_recorded_files_through_the_trash_and_guard() {
    use std::sync::{Arc, Mutex};
    let d = tmp("hd-own");
    let (s, mut coll) = svc(&d);
    let web = d.join("Web");
    std::fs::create_dir_all(&web).unwrap();
    std::fs::write(web.join("a.xmp"), b"theirs").unwrap(); // someone else's sidecar, named like ours
    std::fs::write(web.join("b.jpg"), b"published").unwrap();
    std::fs::write(web.join("b.xmp"), b"ours").unwrap();
    std::fs::write(web.join("orig.jpg"), b"an original").unwrap();
    std::fs::write(web.join("other.xmp"), b"not b's").unwrap(); // a bad record names it below
    coll.sidecars.insert("b.jpg".into(), vec!["b.xmp".into(), "../escape.xmp".into(), "other.xmp".into()]);
    let binned = Arc::new(Mutex::new(Vec::<String>::new()));
    let bin = binned.clone();
    let mut hd = HardDrive::open(&s, &coll)
        .unwrap()
        .with_guard(Arc::new(|p: &std::path::Path| {
            if p.file_name().is_some_and(|n| n == "orig.jpg" || n == "orig.xmp") { Err("the original of orig.jpg".into()) } else { Ok(()) }
        }))
        .with_trash(Arc::new(move |p: &std::path::Path| {
            bin.lock().unwrap().push(p.file_name().unwrap().to_string_lossy().to_string());
            std::fs::remove_file(p).map_err(|e| e.to_string())
        }));
    std::fs::write(web.join("a.jpg"), b"published").unwrap();
    hd.remove("a.jpg").unwrap();
    assert_eq!(std::fs::read(web.join("a.xmp")).unwrap(), b"theirs", "no record: the stem's .xmp is not the collection's");
    hd.remove("b.jpg").unwrap();
    assert_eq!(*binned.lock().unwrap(), vec!["a.jpg", "b.xmp", "b.jpg"], "recorded files only (a bad record is ignored), to the trash");
    assert_eq!(std::fs::read(web.join("other.xmp")).unwrap(), b"not b's", "a record naming another file's sidecar is ignored");
    // a file the guard protects is never removed, and the removal still succeeds
    hd.remove("orig.jpg").unwrap();
    assert_eq!(std::fs::read(web.join("orig.jpg")).unwrap(), b"an original");
    // nor written over, nor given a sidecar
    let side = [("xmp", b"<x/>".to_vec())];
    let up = Upload { photo: PhotoId(1), file_name: "orig.jpg", bytes: b"an original", sidecars: &side, previous: None };
    assert!(hd.publish(&up).unwrap_err().to_string().contains("original"));
    assert!(!web.join("orig.xmp").exists());
    // what it writes is recorded for the next removal
    let up = Upload { photo: PhotoId(2), file_name: "c.jpg", bytes: b"c", sidecars: &side, previous: None };
    assert_eq!(hd.publish(&up).unwrap().sidecars, vec!["c.xmp".to_string()]);
}

#[test]
fn hard_drive_never_leaves_its_folder() {
    let d = tmp("escape");
    std::fs::write(d.join("keep.jpg"), b"k").unwrap();
    let (s, coll) = svc(&d);
    let mut hd = HardDrive::open(&s, &coll).unwrap();
    assert!(hd.remove("../keep.jpg").is_err());
    assert!(hd.remove("..").is_err());
    assert!(d.join("keep.jpg").exists());
    let up = Upload { photo: PhotoId(1), file_name: "../x.jpg", bytes: b"x", sidecars: &[], previous: None };
    // separators are replaced: it lands inside the folder
    assert_eq!(hd.publish(&up).unwrap().remote_id, ".._x.jpg");
    assert!(hd.folder().join(".._x.jpg").exists());
    assert_eq!(safe_name(" .. "), None);
    assert_eq!(safe_name("a/b"), Some("a_b".into()));
    let bad = CollectionConfig { album: AlbumId(2), folder: "..".into(), sidecars: Default::default() };
    assert!(HardDrive::open(&s, &bad).is_err());
    let (mut nodir, _) = svc(&d);
    nodir.settings = json!({});
    assert!(matches!(HardDrive::open(&nodir, &coll), Err(PublishError::Config(_))));
    let mut other = s.clone();
    other.kind = "flickr".into();
    assert!(matches!(open_service(&other, &coll), Err(PublishError::Unsupported(_))));
}

#[test]
fn republish_with_a_new_extension_keeps_the_new_sidecar() {
    let d = tmp("ext");
    let (s, coll) = svc(&d);
    let mut hd = open_service(&s, &coll).unwrap();
    let old = [("xmp", b"<old/>".to_vec())];
    let up = Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one", sidecars: &old, previous: None };
    hd.publish(&up).unwrap();
    let new = [("xmp", b"<new/>".to_vec())];
    let up = Upload { photo: PhotoId(1), file_name: "a.png", bytes: b"two", sidecars: &new, previous: Some("a.jpg") };
    hd.publish(&up).unwrap();
    assert!(!d.join("Web/a.jpg").exists());
    assert_eq!(std::fs::read(d.join("Web/a.png")).unwrap(), b"two");
    assert_eq!(std::fs::read(d.join("Web/a.xmp")).unwrap(), b"<new/>", "the new sidecar must survive removing the old file");
}

#[test]
fn hard_drive_same_name_photos_dont_overwrite_each_other() {
    let d = tmp("hd-clash");
    let (s, coll) = svc(&d);
    let mut hd = open_service(&s, &coll).unwrap();
    let side = [("xmp", b"<x/>".to_vec())];
    let a = hd.publish(&Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one", sidecars: &side, previous: None }).unwrap();
    let b = hd.publish(&Upload { photo: PhotoId(2), file_name: "a.jpg", bytes: b"two", sidecars: &side, previous: None }).unwrap();
    assert_eq!(a.remote_id, "a.jpg");
    assert_eq!(b.remote_id, "a-2.jpg");
    assert_eq!(std::fs::read(d.join("Web/a.jpg")).unwrap(), b"one");
    assert_eq!(std::fs::read(d.join("Web/a-2.jpg")).unwrap(), b"two");
    assert!(d.join("Web/a-2.xmp").exists());
    // re-publishing each photo keeps its own name and replaces only its own file
    let b2 = hd.publish(&Upload { photo: PhotoId(2), file_name: "a.jpg", bytes: b"two!", sidecars: &side, previous: Some("a-2.jpg") }).unwrap();
    assert_eq!(b2.remote_id, "a-2.jpg");
    let a2 = hd.publish(&Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one!", sidecars: &side, previous: Some("a.jpg") }).unwrap();
    assert_eq!(a2.remote_id, "a.jpg");
    assert_eq!(std::fs::read(d.join("Web/a.jpg")).unwrap(), b"one!");
    assert_eq!(std::fs::read(d.join("Web/a-2.jpg")).unwrap(), b"two!");
    // a different extension whose sidecar would clobber another photo's sidecar also moves aside
    let c = hd.publish(&Upload { photo: PhotoId(3), file_name: "a.tif", bytes: b"three", sidecars: &side, previous: None }).unwrap();
    assert_eq!(c.remote_id, "a-3.tif");
}

// #1376: a run stopped before it recorded what it sent leaves the files; the next run adopts a file of the wanted
// name holding exactly the same bytes instead of writing `a-2.jpg` beside it, and still steps aside for another photo's
#[test]
fn hard_drive_adopts_a_file_an_interrupted_run_left() {
    let d = tmp("hd-interrupted");
    let (s, coll) = svc(&d);
    let mut hd = open_service(&s, &coll).unwrap();
    let side = [("xmp", b"<x/>".to_vec())];
    hd.publish(&Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one", sidecars: &side, previous: None }).unwrap();
    // the link was never recorded: the next run sends it again with no previous name
    let again = hd.publish(&Upload { photo: PhotoId(1), file_name: "a.jpg", bytes: b"one", sidecars: &side, previous: None }).unwrap();
    assert_eq!(again.remote_id, "a.jpg");
    assert!(!d.join("Web/a-2.jpg").exists(), "no duplicate");
    // other bytes under the same name are another photo's: they stay
    let other = hd.publish(&Upload { photo: PhotoId(2), file_name: "a.jpg", bytes: b"onf", sidecars: &side, previous: None }).unwrap();
    assert_eq!(other.remote_id, "a-2.jpg");
    assert_eq!(std::fs::read(d.join("Web/a.jpg")).unwrap(), b"one");
}

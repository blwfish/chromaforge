//! JSON has no NaN or infinity: `serde_json` writes them as `null`, which a number field can't read back. An op
//! holding one is refused before it changes the catalog, and the writers of the whole catalog (the v4 checkpoint, the
//! JSON snapshot) refuse to write one that got in another way, so the next open never drops a photo for it.

use std::path::Path;
use std::sync::Arc;

use lcc_develop::DevelopSettings;

use crate::store::MemStore;
use crate::tests_v4::{Scratch, fill};
use crate::*;

fn open(dir: &Path) -> (Journal, Catalog, LoadReport) {
    Journal::open(Box::new(FsStore::open(dir).unwrap())).unwrap()
}

fn nan_settings() -> Arc<DevelopSettings> {
    let mut d = DevelopSettings::default();
    d.light.exposure = f64::NAN;
    Arc::new(d)
}

/// A library of 3 photos, checkpointed.
fn library(tag: &str) -> Scratch {
    let s = Scratch::new(tag);
    let (mut j, mut c, _) = open(s.path());
    fill(&mut j, &mut c, 3);
    j.snapshot(&c).unwrap();
    s
}

/// Put `settings` on photo `id` in memory without an op: how a value that never went through
/// [`Catalog::apply`] would reach a writer.
fn force_develop(c: &mut Catalog, id: PhotoId, settings: Arc<DevelopSettings>) {
    let p = c.photos.get_mut(&id).unwrap();
    Arc::make_mut(p).develop = settings;
}

#[test]
fn an_edit_holding_a_non_finite_number_is_refused_before_it_changes_anything() {
    let s = library("nan-apply");
    let (_j, mut c, _) = open(s.path());
    let id = c.photos().next().unwrap().id;
    let before = c.photo(id).unwrap().develop.clone();
    let revision = c.revision;
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut d = DevelopSettings::default();
        d.light.exposure = v;
        let e = c.apply(Op::SetDevelop { id, settings: Arc::new(d), label: "Exposure".into(), edited: None }).unwrap_err();
        assert!(e.to_string().contains("exposure") && e.to_string().contains("not a finite number"), "{e}");
    }
    // inside a batch too
    let batch = Op::Batch { ops: vec![Op::SetDevelop { id, settings: nan_settings(), label: "Exposure".into(), edited: None }] };
    assert!(c.apply(batch).is_err());
    assert_eq!(c.photo(id).unwrap().develop, before, "the photo is as it was");
    assert_eq!(c.revision, revision, "nothing changed");
}

/// What the review's proof of concept did: the edit used to stay applied in memory (only the log refused it), and
/// the checkpoint at close wrote it as `null`; the next open had 2 of the 3 photos and said nothing.
#[test]
fn a_non_finite_edit_never_drops_a_photo_at_the_next_open() {
    let s = library("nan-reopen");
    let (mut j, mut c, _) = open(s.path());
    let id = c.photos().next().unwrap().id;
    let op = Op::SetDevelop { id, settings: nan_settings(), label: "Exposure".into(), edited: None };
    assert!(c.apply(op).is_err());
    j.snapshot(&c).unwrap();
    drop(j);
    let (_j, back, r) = open(s.path());
    assert_eq!((back.len(), r.failed), (3, 0));
    assert!(back.photo(id).unwrap().develop.light.exposure.is_finite());
}

/// The checkpoint refuses a photo that holds one anyway (it got in without an op): nothing is written, the store
/// keeps the photo as it was, and the next open has every photo.
#[test]
fn the_checkpoint_never_writes_a_non_finite_number() {
    let s = library("nan-checkpoint");
    let (mut j, mut c, _) = open(s.path());
    let id = c.photos().next().unwrap().id;
    force_develop(&mut c, id, nan_settings());
    let e = j.snapshot(&c).unwrap_err();
    assert!(e.to_string().contains("not a finite number"), "{e}");
    drop(j);
    let (_j, back, r) = open(s.path());
    assert_eq!((back.len(), r.failed), (3, 0), "every photo opens");
    assert!(back.photo(id).unwrap().develop.light.exposure.is_finite());
}

/// The same for the catalog's head record (here a saved location), which a load that can't read it refuses whole.
#[test]
fn the_checkpoint_never_writes_a_non_finite_head() {
    let s = library("nan-head");
    let (mut j, mut c, _) = open(s.path());
    let home = lcc_geo::SavedLocation { name: "home".into(), lat: 1.0, lon: 2.0, radius: f64::NAN, private: false };
    c.saved_locations.insert("home".into(), home);
    assert!(j.snapshot(&c).is_err());
    drop(j);
    let (_j, back, _) = open(s.path());
    assert_eq!(back.len(), 3, "the library still opens");
}

/// The JSON snapshot (the web's store) refuses too, and the snapshot it would have replaced still opens.
#[test]
fn the_json_snapshot_never_writes_a_non_finite_number() {
    let m = MemStore::default();
    let (mut j, mut c, _) = Journal::open(Box::new(m.clone())).unwrap();
    fill(&mut j, &mut c, 3);
    j.snapshot(&c).unwrap();
    let id = c.photos().next().unwrap().id;
    force_develop(&mut c, id, nan_settings());
    assert!(j.snapshot(&c).is_err());
    drop(j);
    let (_j, back, _) = Journal::open(Box::new(m)).unwrap();
    assert_eq!(back.len(), 3);
}

/// A number the file's metadata didn't give (an aperture of 0/0) is a NaN directly in an optional field: it is
/// written as `null` and reads back as "not known". Such a photo is still added and saved, as before.
#[test]
fn a_non_finite_number_in_an_optional_field_is_saved_as_unknown() {
    let s = library("nan-optional");
    let (mut j, mut c, _) = open(s.path());
    let mut p = crate::tests_v4::photo(&mut c, 9);
    p.meta.aperture = Some(f32::NAN);
    let id = p.id;
    let op = Op::AddPhoto { photo: Box::new(p) };
    c.apply(op.clone()).unwrap();
    j.append(&[op]).unwrap();
    j.snapshot(&c).unwrap();
    drop(j);
    let (_j, back, r) = open(s.path());
    assert_eq!((back.len(), r.failed), (4, 0));
    assert_eq!(back.photo(id).unwrap().meta.aperture, None);
}

/// The same for a number outside the develop settings (a clip's length).
#[test]
fn the_checkpoint_never_writes_a_non_finite_number_outside_the_develop_settings() {
    let s = library("nan-video");
    let (mut j, mut c, _) = open(s.path());
    let id = c.photos().next().unwrap().id;
    let video = VideoMeta {
        duration: f64::NAN,
        fps_num: 30,
        fps_den: 1,
        width: 4,
        height: 3,
        codec: "h264".into(),
        has_audio: false,
        trim_in: 0.0,
        trim_out: None,
        poster_time: None,
    };
    Arc::make_mut(c.photos.get_mut(&id).unwrap()).video = Some(video);
    let e = j.snapshot(&c).unwrap_err();
    assert!(e.to_string().contains("duration"), "{e}");
    drop(j);
    let (_j, back, r) = open(s.path());
    assert_eq!((back.len(), r.failed), (3, 0));
}

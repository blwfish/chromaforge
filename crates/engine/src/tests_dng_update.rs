//! Metadata ▸ Update DNG Preview & Metadata (`photo.updateDngPreview`): the file is rewritten with a
//! preview and the photo's XMP and the raw stays what it was; files that must not be touched are left
//! byte for byte as they were.

use std::path::PathBuf;

use serde_json::{Value, json};

use crate::Session;
use crate::tests_xmp::{synthetic_dng_with, temp_dir};

/// A session holding the one DNG `bytes` written to a scratch folder.
fn with_dng(tag: &str, bytes: &[u8]) -> (Session, lcc_testkit::TempDir, PathBuf) {
    let dir = temp_dir(tag);
    let file = dir.join("shot.dng");
    std::fs::write(&file, bytes).unwrap();
    let mut s = Session::new().with_fs();
    s.execute("library.import", &json!({"paths": [file.to_string_lossy()]})).unwrap();
    (s, dir, file)
}

fn pixels(file: &[u8]) -> Vec<u16> {
    match lcc_raw::decode(file).unwrap().data {
        lcc_raw::RawData::U16(v) => v,
        other => panic!("unexpected data {other:?}"),
    }
}

fn xmp_of(file: &[u8]) -> String {
    String::from_utf8(lcc_raw::dngupdate::embedded_xmp(file).unwrap().expect("an embedded XMP packet")).unwrap()
}

#[test]
fn the_dng_gets_a_preview_and_the_current_xmp_merged_and_keeps_its_raw() {
    // (another writer's XMP is already in the file)
    let xmp = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
      <rdf:Description rdf:about="" xmlns:foo="http://example.com/foo/" foo:Keep="precious"/></rdf:RDF></x:xmpmeta>"#;
    let original = synthetic_dng_with(Some(xmp), lcc_meta::Metadata::default());
    let (mut s, dir, file) = with_dng("dng-update", &original);
    s.execute("photo.rate", &json!({"rating": 4})).unwrap();
    // bad parameters are reported, and change nothing
    for bad in [json!({"longEdge": 10}), json!({"longEdge": "big"}), json!({"quality": 0}), json!({"quality": 101}), json!({"ids": ["x"]})] {
        assert!(s.execute("photo.updateDngPreview", &bad).is_err(), "{bad}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), original, "a refused update leaves the file alone");
    let r = s.execute("photo.updateDngPreview", &json!({})).unwrap();
    assert_eq!(r["updated"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(r["updated"][0]["replacedPreviews"], 0);
    let after = std::fs::read(&file).unwrap();
    assert_eq!(pixels(&after), pixels(&original), "the raw decodes the same");
    assert!(xmp_of(&after).contains("Rating"), "the rating is in the embedded XMP");
    assert!(xmp_of(&after).contains("precious"), "the other writer's XMP in the file is kept: {}", xmp_of(&after));
    let tiff = lcc_tiff::Tiff::parse(&after).unwrap();
    let preview = tiff.ifds[0].sub_ifds.last().expect("a preview IFD");
    let (at, len) = (preview.u32(lcc_tiff::tags::STRIP_OFFSETS).unwrap() as usize, preview.u32(lcc_tiff::tags::STRIP_BYTE_COUNTS).unwrap() as usize);
    assert_eq!(after.get(at..at + 2), Some(&[0xFF, 0xD8][..]), "a JPEG preview");
    assert!(len > 100);
    // the update leaves no activity row behind, and a second one replaces the preview
    assert!(s.activity.list().is_empty());
    let again = s.execute("photo.updateDngPreview", &json!({"longEdge": 512})).unwrap();
    assert_eq!(again["updated"][0]["replacedPreviews"], 1, "{again}");
    let twice = std::fs::read(&file).unwrap();
    assert_eq!(lcc_tiff::Tiff::parse(&twice).unwrap().ifds[0].sub_ifds.len(), 1);
    assert_eq!(pixels(&twice), pixels(&original));
    let r: Value = s.execute("photo.updateDngPreview", &json!({"quality": 60})).unwrap();
    assert_eq!(r["failed"].as_array().map(Vec::len), Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn files_it_must_not_touch_are_left_byte_for_byte() {
    // a photo that is not a DNG
    let mut demo = Session::with_demo();
    let e = demo.execute("photo.updateDngPreview", &json!({})).unwrap_err().to_string();
    assert!(e.contains("DNG"), "{e}");
    // Written by Adobe software
    let good = synthetic_dng_with(None, lcc_meta::Metadata::default());
    let sw = lcc_tiff::Tiff::parse(&good).unwrap().ifds[0].get(lcc_tiff::tags::SOFTWARE).unwrap().offset as usize;
    let mut adobe = good.clone();
    adobe[sw..sw + 5].copy_from_slice(b"Adobe");
    let (mut s, dir, file) = with_dng("dng-update-adobe", &adobe);
    let e = s.execute("photo.updateDngPreview", &json!({})).unwrap_err().to_string();
    assert!(e.contains("Adobe"), "{e}");
    assert_eq!(std::fs::read(&file).unwrap(), adobe);
    assert!(s.activity.list().is_empty());
    let _ = std::fs::remove_dir_all(&dir);

    // an embedded XMP packet that cannot be read
    let (mut s, dir, file) = with_dng("dng-update-badxmp", &synthetic_dng_with(Some("<x:xmpmeta><unclosed"), lcc_meta::Metadata::default()));
    let before = std::fs::read(&file).unwrap();
    let e = s.execute("photo.updateDngPreview", &json!({})).unwrap_err().to_string();
    assert!(e.contains("XMP"), "{e}");
    assert_eq!(std::fs::read(&file).unwrap(), before);
    let _ = std::fs::remove_dir_all(&dir);

    // a damaged file: an error, no panic, no change
    let mut cut = good.clone();
    cut.truncate(good.len() / 2);
    let (mut s, dir, file) = with_dng("dng-update-cut", &good);
    std::fs::write(&file, &cut).unwrap();
    assert!(s.execute("photo.updateDngPreview", &json!({})).is_err());
    assert_eq!(std::fs::read(&file).unwrap(), cut);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A new file that doesn't read back as written (a failing drive, a flaky network volume) never replaces the
/// original: the photo is reported failed and the DNG is left byte for byte as it was.
#[test]
fn a_dng_whose_new_file_reads_back_different_is_left_as_it_was() {
    let original = synthetic_dng_with(None, lcc_meta::Metadata::default());
    let (mut s, dir, file) = with_dng("dng-update-read-back", &original);
    s.execute("photo.rate", &json!({"rating": 3})).unwrap();
    let r = {
        let _spoil = lcc_catalog::safe_file::spoil_written_files();
        s.execute("photo.updateDngPreview", &json!({}))
    };
    let e = r.expect_err("a single photo that failed is an error");
    assert!(e.to_string().contains("read back different"), "{e}");
    assert_eq!(std::fs::read(&file).unwrap(), original, "the original is left byte for byte");
    // with the volume working again, the same update goes through
    let ok = s.execute("photo.updateDngPreview", &json!({})).unwrap();
    assert_eq!(ok["updated"].as_array().map(Vec::len), Some(1), "{ok}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A virtual copy shares its master's file; its edits live only in the catalog. Updating it must not write the
/// copy's XMP and render into the master's original (every other writer of files refuses virtual copies): the copy
/// is skipped with the reason, and with only copies selected the command is not available.
#[test]
fn a_virtual_copy_never_writes_into_its_masters_dng() {
    let (mut s, dir, file) = with_dng("dng-update-copy", &synthetic_dng_with(None, lcc_meta::Metadata::default()));
    let master = s.catalog.photos().next().unwrap().id;
    s.execute("photo.virtualCopy", &json!({"ids": [master.0]})).unwrap();
    let copy = s.catalog.photos().find(|p| p.copy_of == Some(master)).expect("a virtual copy").id;
    s.execute("photo.rate", &json!({"ids": [copy.0], "rating": 5})).unwrap();
    let before = std::fs::read(&file).unwrap();
    // named: refused, the file untouched
    let e = s.execute("photo.updateDngPreview", &json!({"ids": [copy.0]})).unwrap_err().to_string();
    assert!(e.contains("virtual copy"), "{e}");
    assert_eq!(std::fs::read(&file).unwrap(), before, "the master's DNG is left byte for byte");
    // selected: the menu item is not available
    s.selection.ids = vec![copy];
    s.selection.active = Some(copy);
    let e = s.execute("photo.updateDngPreview", &json!({})).unwrap_err();
    assert!(matches!(e, crate::EngineError::Disabled(..)), "{e:?}");
    assert_eq!(std::fs::read(&file).unwrap(), before);

    // master and copy together: the master's own edit is written, once; the copy is skipped
    s.execute("photo.rate", &json!({"ids": [master.0], "rating": 2})).unwrap();
    let r = s.execute("photo.updateDngPreview", &json!({"ids": [master.0, copy.0]})).unwrap();
    let ids = |k: &str| r[k].as_array().unwrap().iter().map(|v| v["id"].as_u64().unwrap()).collect::<Vec<_>>();
    assert_eq!((ids("updated"), ids("skipped"), ids("failed")), (vec![master.0], vec![copy.0], vec![]), "{r}");
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("virtual copy"), "{r}");
    let xmp = xmp_of(&std::fs::read(&file).unwrap());
    assert!(xmp.contains("Rating=\"2\"") || xmp.contains("<xmp:Rating>2<"), "the master's rating: {xmp}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Two catalog records of one file (whatever made them) rewrite it once: the first named; the other is skipped.
#[test]
fn one_file_is_updated_once_however_many_photos_name_it() {
    let (mut s, dir, _file) = with_dng("dng-update-same-file", &synthetic_dng_with(None, lcc_meta::Metadata::default()));
    let first = s.catalog.photos().next().unwrap().id;
    let mut twin = (**s.catalog.photo(first).unwrap()).clone();
    twin.id = s.catalog.alloc_photo_id();
    let second = twin.id;
    s.catalog.apply(lcc_catalog::Op::AddPhoto { photo: Box::new(twin) }).unwrap();
    let r = s.execute("photo.updateDngPreview", &json!({"ids": [first.0, second.0]})).unwrap();
    assert_eq!(r["updated"].as_array().map(Vec::len), Some(1), "{r}");
    assert_eq!(r["skipped"][0]["id"].as_u64(), Some(second.0), "{r}");
    assert!(r["skipped"][0]["reason"].as_str().unwrap().contains("same file"), "{r}");
    let _ = std::fs::remove_dir_all(&dir);
}

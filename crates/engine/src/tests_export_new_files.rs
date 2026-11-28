//! FX-143: under Unique and Skip an export never replaces a file, also not one that appears in
//! the folder while it runs.

use serde_json::json;

use crate::Session;
use crate::export::{ExportFormat, ExportOptions};
use crate::tests_xmp::temp_dir;

fn jpeg() -> Vec<u8> {
    let pixels = [96u8; 12];
    let image = lcc_codecs::EncodeImage::new(2, 2, 3, lcc_codecs::Samples::U8(&pixels));
    lcc_codecs::encode_jpeg(&image, 90, lcc_codecs::ChromaSubsampling::S444, &Default::default()).unwrap()
}

/// FX-143: Unique and Skip never replace a file that appears after the export looked (another
/// export into the folder, a sync client). `exists` here writes "their" file right after it says
/// the name is free, as such a program would in the moment before the rename. Before, the export's
/// rename replaced it and reported success.
#[test]
fn unique_and_skip_never_replace_a_file_that_appears_meanwhile() {
    use crate::export::{Conflict, Destination, export_batch, write_file};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    let dir = temp_dir("appears-meanwhile");
    let src = dir.join("src").join("IMG_7.jpg");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, jpeg()).unwrap();
    let mut s = Session::new().with_fs();
    s.execute("library.import", &json!({"paths": [src.to_string_lossy()]})).unwrap();
    let id = s.catalog.photos().next().unwrap().id;
    // `ext`: the one name that appears ("jpg": the file itself; "xmp": an Original's sidecar)
    let run = |s: &mut Session, conflict: Conflict, format: ExportFormat, ext: &str, tag: &str| {
        let out = dir.join(tag);
        let o = ExportOptions { conflict, format, ..Default::default() };
        let to = Destination::on_disk(out.to_string_lossy().into(), None);
        let planted: RefCell<Option<PathBuf>> = RefCell::new(None);
        let exists = |p: &str| {
            let path = Path::new(p);
            if planted.borrow().is_none() && path.extension().is_some_and(|x| x == ext) {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, b"theirs").unwrap();
                *planted.borrow_mut() = Some(path.to_path_buf());
                return false;
            }
            path.exists()
        };
        let files = export_batch(s, &[id], &o, &to, &mut write_file, &exists).unwrap();
        let theirs = planted.into_inner().expect("a file appeared");
        (files, theirs, out)
    };
    let names = |out: &Path| {
        let mut v: Vec<String> = std::fs::read_dir(out).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };

    // Unique: the next free name; theirs kept
    let (files, theirs, out) = run(&mut s, Conflict::Unique, ExportFormat::Jpeg, "jpg", "unique");
    assert_eq!(std::fs::read(&theirs).unwrap(), b"theirs", "the file that appeared is kept");
    let ours = files[0]["path"].as_str().unwrap();
    assert!(ours.ends_with("IMG_7-2.jpg"), "{files:?}");
    assert!(std::fs::read(ours).unwrap().starts_with(&[0xff, 0xd8]), "ours is the export");
    assert_eq!(names(&out), ["IMG_7-2.jpg", "IMG_7.jpg"], "no temp file left");
    // Skip: skipped; theirs kept
    let (files, theirs, out) = run(&mut s, Conflict::Skip, ExportFormat::Jpeg, "jpg", "skip");
    assert_eq!(files[0]["skipped"].as_str(), Some(&*theirs.to_string_lossy()), "{files:?}");
    assert_eq!(std::fs::read(&theirs).unwrap(), b"theirs");
    assert_eq!(names(&out), ["IMG_7.jpg"]);
    // an Original's sidecar that appears: the photo moves to the next name with both its files,
    // and the copy it wrote under the first name is gone again
    let (files, theirs, out) = run(&mut s, Conflict::Unique, ExportFormat::Original, "xmp", "unique-sidecar");
    assert_eq!(std::fs::read(&theirs).unwrap(), b"theirs");
    assert!(files[0]["path"].as_str().unwrap().ends_with("IMG_7-2.jpg"), "{files:?}");
    assert_eq!(names(&out), ["IMG_7-2.jpg", "IMG_7-2.xmp", "IMG_7.xmp"]);
    let (files, _, out) = run(&mut s, Conflict::Skip, ExportFormat::Original, "xmp", "skip-sidecar");
    assert!(files[0]["skipped"].is_string(), "{files:?}");
    assert_eq!(names(&out), ["IMG_7.xmp"], "skipped with all its files");
    // Overwrite still replaces what it finds (the policy the user chose)
    let (files, theirs, _) = run(&mut s, Conflict::Overwrite, ExportFormat::Jpeg, "jpg", "overwrite");
    assert_eq!(files[0]["path"].as_str(), Some(&*theirs.to_string_lossy()));
    assert!(std::fs::read(&theirs).unwrap().starts_with(&[0xff, 0xd8]));
}

/// FX-143: a movie placed as a new file never replaces one that is there; it stays in its temp
/// file to be placed elsewhere.
#[test]
fn a_movie_placed_as_a_new_file_never_replaces_one() {
    use crate::export_video::TempFile;
    let dir = temp_dir("movie-new");
    let theirs = dir.join("clip.mp4");
    std::fs::write(&theirs, b"theirs").unwrap();
    let tmp = dir.join("render.tmp");
    std::fs::write(&tmp, b"ours").unwrap();
    let movie = TempFile(tmp.clone());
    assert_eq!(movie.move_new_to(&theirs.to_string_lossy()), Ok(false));
    assert_eq!(std::fs::read(&theirs).unwrap(), b"theirs");
    let free = dir.join("sub").join("clip-2.mp4");
    assert_eq!(movie.move_new_to(&free.to_string_lossy()), Ok(true));
    assert_eq!(std::fs::read(&free).unwrap(), b"ours");
    assert!(!tmp.exists(), "moved, not copied");
}

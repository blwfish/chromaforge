//! What an export lets out of the library, for every kind of file it writes: an Original's
//! sidecar and a raw DNG follow the Metadata choice and the keywords' "include on export" option
//! as a rendered file does (FX-137).

use lcc_catalog::keywords::KeywordInfo;
use lcc_catalog::{Op, PhotoId};
use serde_json::json;

use crate::Session;
use crate::export::{ExportFormat, ExportOptions, MetadataPolicy, export_photo};
use crate::tests_xmp::{synthetic_dng_with, temp_dir};

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn jpeg() -> Vec<u8> {
    let pixels = [96u8; 12];
    let image = lcc_codecs::EncodeImage::new(2, 2, 3, lcc_codecs::Samples::U8(&pixels));
    lcc_codecs::encode_jpeg(&image, 90, lcc_codecs::ChromaSubsampling::S444, &Default::default()).unwrap()
}

fn import(s: &mut Session, path: &std::path::Path) -> PhotoId {
    s.execute("library.import", &json!({"paths": [path.to_string_lossy()]})).unwrap();
    s.catalog.photos().find(|p| p.file_name == path.file_name().unwrap().to_string_lossy()).map(|p| p.id).unwrap()
}

/// `secretkw` marked "don't include on export", `publickw`, a person keyword `Alice` (under
/// `People`), a title, a caption, a copyright and a rating on photo `id`.
fn private_metadata(s: &mut Session, id: PhotoId) {
    s.catalog
        .apply(Op::SetKeyword { path: "secretkw".into(), info: Some(KeywordInfo { include_on_export: false, ..KeywordInfo::default() }) })
        .unwrap();
    s.catalog.apply(Op::SetKeyword { path: "People|Alice".into(), info: Some(KeywordInfo { person: true, ..KeywordInfo::default() }) }).unwrap();
    let mut meta = s.catalog.photo(id).unwrap().meta.clone();
    meta.keywords = vec!["secretkw".into(), "publickw".into(), "People|Alice".into()];
    meta.title = "KwSecretTitle".into();
    meta.caption = "KwPrivateCaption".into();
    meta.copyright = "KwCopyright".into();
    s.catalog.apply(Op::SetMeta { id, meta: Box::new(meta) }).unwrap();
    s.catalog.apply(Op::SetRating { id, rating: 4 }).unwrap();
}

/// FX-137: an Original's `.xmp` sidecar follows the Metadata choice and the keyword flags (the
/// review's PoC `poc_original_sidecar_follows_metadata_policy_and_keyword_flags`); the edits go
/// under every choice, since carrying them is what the sidecar is for.
#[test]
fn an_originals_sidecar_follows_the_metadata_choice_and_keyword_flags() {
    let dir = temp_dir("kw-original");
    let src = dir.join("IMG_1.jpg");
    std::fs::write(&src, jpeg()).unwrap();
    let mut s = Session::new().with_fs();
    let id = import(&mut s, &src);
    private_metadata(&mut s, id);
    let sidecar = |s: &mut Session, o: &ExportOptions| -> Vec<u8> {
        let e = export_photo(s, id, o, 1).unwrap();
        assert_eq!(e.sidecars.len(), 1);
        e.sidecars[0].1.clone()
    };
    for policy in [MetadataPolicy::None, MetadataPolicy::Copyright, MetadataPolicy::CopyrightContact] {
        let x = sidecar(&mut s, &ExportOptions { format: ExportFormat::Original, metadata: policy, ..Default::default() });
        for leak in ["secretkw", "publickw", "KwSecretTitle", "KwPrivateCaption", "xmp:Rating"] {
            assert!(!contains(&x, leak), "{policy:?}: the sidecar holds {leak}");
        }
        assert!(contains(&x, "lc:settings") && contains(&x, "exposure"), "{policy:?}: the edits go with the original");
        assert_eq!(contains(&x, "KwCopyright"), policy != MetadataPolicy::None, "{policy:?}: copyright as chosen");
    }
    // All: everything but the keyword left out of export
    let x = sidecar(&mut s, &ExportOptions { format: ExportFormat::Original, ..Default::default() });
    assert!(!contains(&x, "secretkw"), "the keyword marked not-for-export");
    for kept in ["publickw", "Alice", "KwSecretTitle", "KwPrivateCaption", "KwCopyright", "lc:settings"] {
        assert!(contains(&x, kept), "All keeps {kept}");
    }
}

/// FX-137: a raw exported as DNG embeds the same packet (the review's PoC
/// `poc_dng_export_follows_metadata_policy_and_keyword_flags`).
#[test]
fn a_dng_export_follows_the_metadata_choice_and_keyword_flags() {
    let dir = temp_dir("kw-dng");
    let src = dir.join("Shot.dng");
    std::fs::write(&src, synthetic_dng_with(None, Default::default())).unwrap();
    let mut s = Session::new().with_fs();
    let id = import(&mut s, &src);
    private_metadata(&mut s, id);
    let e = export_photo(&mut s, id, &ExportOptions { format: ExportFormat::Dng, metadata: MetadataPolicy::None, ..Default::default() }, 1).unwrap();
    for leak in ["secretkw", "publickw", "KwSecretTitle", "KwPrivateCaption", "KwCopyright"] {
        assert!(!contains(&e.bytes, leak), "metadata None: the DNG holds {leak}");
    }
    assert!(contains(&e.bytes, "lc:settings"), "the edits are embedded");
    let e = export_photo(&mut s, id, &ExportOptions { format: ExportFormat::Dng, ..Default::default() }, 1).unwrap();
    assert!(
        !contains(&e.bytes, "secretkw") && contains(&e.bytes, "publickw") && contains(&e.bytes, "KwPrivateCaption"),
        "All, without the excluded keyword"
    );
}

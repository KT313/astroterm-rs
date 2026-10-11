use super::*;

#[test]
fn immutable_blobs_survive_reference_replacement_and_corruption_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    let old = store.publish("block", b"old", false).unwrap();
    store.publish("block", b"new", false).unwrap();
    assert_eq!(store.read_object(&old.hash).unwrap(), b"old");
    store.check_unchanged(&old.hash, old.stamp).unwrap();
    assert_eq!(store.read("block").unwrap().unwrap(), b"new");

    let object = dir.path().join("objects").join(digest(b"new"));
    fs::write(&object, b"bad").unwrap();
    let error = store.read("block").err().unwrap().to_string();
    assert!(error.contains("checksum mismatch") && error.contains(&object.display().to_string()));
    store.publish("block", b"new", false).unwrap();
    assert_eq!(store.read("block").unwrap().unwrap(), b"new");

    store.publish("pin", b"identity", true).unwrap();
    assert!(store.publish("pin", b"changed", true).is_err());
    assert_eq!(store.read("pin").unwrap().unwrap(), b"identity");
}

#[test]
fn concurrent_publication_always_points_at_one_complete_blob() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    std::thread::scope(|scope| {
        for byte in [1, 2] {
            let store = &store;
            scope.spawn(move || {
                for _ in 0..8 { store.publish("result", &vec![byte; 4096], false).unwrap(); }
            });
        }
    });
    let bytes = store.read("result").unwrap().unwrap();
    assert_eq!(bytes.len(), 4096);
    assert!(bytes.iter().all(|b| *b == bytes[0]));
}

#[test]
fn final_stamp_check_detects_changes_without_another_content_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = BlobStore::new(dir.path().into());
    let blob = store.publish("block", b"verified input", false).unwrap();
    let verified = store.read_verified("block").unwrap().unwrap();
    assert_eq!(verified.stamp, blob.stamp);
    store.check_unchanged(&blob.hash, blob.stamp).unwrap();
    fs::write(store.object_path(&blob.hash).unwrap(), b"short").unwrap();
    assert!(store.check_unchanged(&blob.hash, blob.stamp).is_err());
}

#[test]
fn locating_uses_only_metadata_and_defers_same_size_same_mtime_corruption() {
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    let saved = store.publish("raw", b"original", false).unwrap();
    let path = store.object_path(&saved.hash).unwrap();
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, b"damaged!").unwrap();
    fs::File::options().write(true).open(&path).unwrap().set_modified(modified).unwrap();
    let reads = store.payload_reads(&saved.hash);
    let located = store.locate("raw").unwrap().unwrap();
    assert_eq!((located.hash.as_str(), located.bytes, located.stamp), (saved.hash.as_str(), 8, saved.stamp));
    assert_eq!(store.payload_reads(&saved.hash), reads);
    store.check_unchanged(&saved.hash, located.stamp).unwrap(); // metadata comparisons cannot prove preexisting payload integrity
    assert!(store.read_object(&saved.hash).unwrap_err().to_string().contains("checksum mismatch"));
    assert_eq!(store.payload_reads(&saved.hash), reads + 1);
}

#[test]
fn locating_rejects_missing_objects_bad_references_and_wrong_lengths() {
    let root = tempfile::tempdir().unwrap();
    let store = BlobStore::new(root.path().into());
    assert!(store.locate("absent").unwrap().is_none());
    let saved = store.publish("raw", b"payload", false).unwrap();
    fs::write(store.object_path(&saved.hash).unwrap(), b"short").unwrap();
    assert!(store.locate("raw").is_err());
    for text in ["malformed", "version=1\nkey='raw'\nhash='../escape'\nbytes=7", "version=2\nkey='raw'\nhash='invalid'\nbytes=7"] {
        fs::write(store.reference("raw"), text).unwrap();
        assert!(store.locate("raw").is_err());
    }
    let reference = Reference { version: 1, key: "raw".into(), hash: "0".repeat(64), bytes: 7 };
    fs::write(store.reference("raw"), toml::to_string(&reference).unwrap()).unwrap();
    assert_eq!(store.locate("raw").err().unwrap().kind(), io::ErrorKind::NotFound);
}

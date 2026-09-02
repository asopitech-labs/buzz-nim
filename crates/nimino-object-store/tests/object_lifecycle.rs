use std::{fs, path::PathBuf};

use nimino_object_store::{LocalObjectStore, ObjectStoreError, MAX_CHUNK_BYTES};
use sha2::{Digest, Sha256};
use uuid::Uuid;

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!("nimino-objects-{}", Uuid::new_v4())))
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[test]
fn resumes_large_partial_and_installs_atomically() {
    let root = TestRoot::new();
    let bytes = (0..(MAX_CHUNK_BYTES * 9 + 17))
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>();
    let expected = digest(&bytes);
    let store = LocalObjectStore::open(&root.0).unwrap();
    let mut offset = 0_u64;
    for chunk in bytes[..MAX_CHUNK_BYTES * 4].chunks(MAX_CHUNK_BYTES) {
        offset = store
            .append_partial("transfer-a", &expected, bytes.len() as u64, offset, chunk)
            .unwrap()
            .offset;
    }
    drop(store);

    let reopened = LocalObjectStore::open(&root.0).unwrap();
    assert_eq!(
        reopened
            .begin_partial("transfer-a", &expected, bytes.len() as u64)
            .unwrap()
            .offset,
        offset
    );
    for chunk in bytes[offset as usize..].chunks(MAX_CHUNK_BYTES) {
        offset = reopened
            .append_partial("transfer-a", &expected, bytes.len() as u64, offset, chunk)
            .unwrap()
            .offset;
    }
    let installed = reopened
        .finish_partial("transfer-a", &expected, bytes.len() as u64)
        .unwrap();
    assert!(installed.installed);
    reopened.verify(&expected, bytes.len() as u64).unwrap();
    assert_eq!(
        reopened
            .read_chunk(&expected, bytes.len() as u64, 11, 37)
            .unwrap(),
        bytes[11..48]
    );
    assert_eq!(reopened.read(&expected, bytes.len() as u64).unwrap(), bytes);
}

#[test]
fn checksum_failure_never_installs_and_existing_object_is_idempotent() {
    let root = TestRoot::new();
    let bytes = b"verified bytes";
    let expected = digest(bytes);
    let store = LocalObjectStore::open(&root.0).unwrap();
    store
        .append_partial("first", &expected, bytes.len() as u64, 0, bytes)
        .unwrap();
    assert!(
        store
            .finish_partial("first", &expected, bytes.len() as u64)
            .unwrap()
            .installed
    );
    store
        .append_partial("second", &expected, bytes.len() as u64, 0, bytes)
        .unwrap();
    assert!(
        !store
            .finish_partial("second", &expected, bytes.len() as u64)
            .unwrap()
            .installed
    );

    let wrong_digest = digest(b"different bytes");
    store
        .append_partial("wrong", &wrong_digest, bytes.len() as u64, 0, bytes)
        .unwrap();
    assert!(matches!(
        store.finish_partial("wrong", &wrong_digest, bytes.len() as u64),
        Err(ObjectStoreError::DigestMismatch { .. })
    ));
    assert!(matches!(
        store.read(&wrong_digest, u64::MAX),
        Err(ObjectStoreError::NotFound)
    ));
}

#[test]
fn chunk_offset_cancel_and_gc_delete_are_bounded_and_idempotent() {
    let root = TestRoot::new();
    let bytes = b"partial";
    let expected = digest(bytes);
    let store = LocalObjectStore::open(&root.0).unwrap();
    store
        .append_partial("transfer-a", &expected, bytes.len() as u64, 0, b"par")
        .unwrap();
    assert!(matches!(
        store.append_partial("transfer-a", &expected, bytes.len() as u64, 0, b"tial"),
        Err(ObjectStoreError::OffsetMismatch { .. })
    ));
    store
        .abort_partial("transfer-a", &expected, bytes.len() as u64)
        .unwrap();
    store
        .abort_partial("transfer-a", &expected, bytes.len() as u64)
        .unwrap();

    store
        .append_partial("complete", &expected, bytes.len() as u64, 0, bytes)
        .unwrap();
    store
        .finish_partial("complete", &expected, bytes.len() as u64)
        .unwrap();
    store.delete(&expected).unwrap();
    store.delete(&expected).unwrap();
    assert!(matches!(
        store.verify(&expected, bytes.len() as u64),
        Err(ObjectStoreError::NotFound)
    ));
}

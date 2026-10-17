//! Persistence of the diff cache.
//!
//! The cache is written as one snapshot when it changed, by a background task
//! and on shutdown. Writes are serialized, so an older snapshot can never
//! replace a newer one. The file is bounded by the cache capacity.
//!
//! Cached diffs decide what gets signed (files, commits, archive hashes), so
//! with a signing key the snapshot is authenticated with HMAC-SHA256 under a
//! key derived from the signing key. A file that was modified by anyone else
//! is ignored, so write access to the data directory is not enough to make
//! the service sign something.
//!
//! File format: the hex MAC (or `-` without a key) on the first line,
//! followed by the JSON snapshot.
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use hmac::{Hmac, KeyInit, Mac};
use moka::future::Cache;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{info, warn};
use zeroize::Zeroizing;

use crate::models::diff::CachedDiff;
use crate::state::AppState;
use crate::utils::fs::{atomic_write, read_limited};
use crate::validation::is_valid_zip_filename;

const SCHEMA_VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const FLUSH_INTERVAL: Duration = Duration::from_secs(60);
const NO_MAC: &str = "-";

#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema: u32,
    saved_at: String,
    entries: Vec<(String, CachedDiff)>,
}

pub struct DiffCacheStore {
    path: PathBuf,
    mac_key: Option<Zeroizing<[u8; 32]>>,
    dirty: AtomicBool,
    write_lock: Mutex<()>,
}

impl DiffCacheStore {
    pub fn new(path: PathBuf, mac_key: Option<Zeroizing<[u8; 32]>>) -> Self {
        Self {
            path,
            mac_key,
            dirty: AtomicBool::new(false),
            write_lock: Mutex::new(()),
        }
    }

    pub fn mark_dirty(&self) {
        self.dirty.store(true, Ordering::Release);
    }

    fn mac(&self, payload: &[u8]) -> String {
        match &self.mac_key {
            Some(key) => {
                let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_ref())
                    .expect("HMAC accepts keys of any length");
                mac.update(payload);
                hex::encode(mac.finalize().into_bytes())
            }
            None => NO_MAC.to_string(),
        }
    }

    fn verify(&self, tag: &str, payload: &[u8]) -> bool {
        match &self.mac_key {
            Some(key) => {
                let Ok(tag) = hex::decode(tag) else {
                    return false;
                };
                let mut mac = <Hmac<Sha256> as KeyInit>::new_from_slice(key.as_ref())
                    .expect("HMAC accepts keys of any length");
                mac.update(payload);
                // Constant-time comparison.
                mac.verify_slice(&tag).is_ok()
            }
            None => tag == NO_MAC,
        }
    }

    /// Loads the persisted entries; the cache expires old ones by their
    /// creation time. Missing, tampered, invalid or outdated files are
    /// ignored and replaced on the next flush.
    pub async fn load_into(&self, cache: &Cache<String, CachedDiff>) {
        let data = match read_limited(&self.path, MAX_FILE_BYTES).await {
            Ok(data) => data,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) => {
                warn!("cannot read diff cache {}: {e}", self.path.display());
                return;
            }
        };
        let Some(split) = data.iter().position(|&b| b == b'\n') else {
            info!(
                "ignoring diff cache in an outdated format: {}",
                self.path.display()
            );
            return;
        };
        let (tag, payload) = (&data[..split], &data[split + 1..]);
        let authentic = std::str::from_utf8(tag).is_ok_and(|tag| self.verify(tag, payload));
        if !authentic {
            warn!(
                "ignoring diff cache {}: it was modified, written with another key or in an outdated format",
                self.path.display()
            );
            return;
        }

        let payload = payload.to_vec();
        let parsed =
            tokio::task::spawn_blocking(move || serde_json::from_slice::<Snapshot>(&payload)).await;
        let snapshot = match parsed {
            Ok(Ok(snapshot)) if snapshot.schema == SCHEMA_VERSION => snapshot,
            _ => {
                info!(
                    "ignoring diff cache in an outdated or invalid format: {}",
                    self.path.display()
                );
                return;
            }
        };

        for (key, mut diff) in snapshot.entries {
            if diff
                .zip
                .as_ref()
                .is_some_and(|zip| !is_valid_zip_filename(&zip.filename))
            {
                diff.zip = None;
            }
            cache.insert(key, diff).await;
        }
        cache.run_pending_tasks().await;
        info!(
            entries = cache.entry_count(),
            "loaded persistent diff cache"
        );
    }

    /// Writes the cache to disk if it changed since the last flush.
    pub async fn flush(&self, cache: &Cache<String, CachedDiff>) {
        let _writing = self.write_lock.lock().await;
        if !self.dirty.swap(false, Ordering::AcqRel) {
            return;
        }
        let entries: Vec<(String, CachedDiff)> = cache
            .iter()
            .map(|(key, diff)| (key.as_ref().clone(), diff))
            .collect();
        let snapshot = Snapshot {
            schema: SCHEMA_VERSION,
            saved_at: chrono::Utc::now().to_rfc3339(),
            entries,
        };
        let serialized = tokio::task::spawn_blocking(move || serde_json::to_vec(&snapshot)).await;
        let result = match serialized {
            Ok(Ok(payload)) => {
                let mut file = self.mac(&payload).into_bytes();
                file.push(b'\n');
                file.extend_from_slice(&payload);
                atomic_write(&self.path, file)
                    .await
                    .map_err(anyhow::Error::from)
            }
            Ok(Err(e)) => Err(e.into()),
            Err(e) => Err(e.into()),
        };
        if let Err(e) = result {
            self.mark_dirty();
            warn!("failed to save diff cache {}: {e:#}", self.path.display());
        }
    }

    pub fn spawn_flusher(state: Arc<AppState>) -> JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(FLUSH_INTERVAL);
            loop {
                ticker.tick().await;
                state.diff_store.flush(&state.caches.diffs).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::diff::{DiffSummary, ZipArtifact};
    use crate::utils::cache::Caches;

    fn diff(created_at: i64, zip_name: Option<&str>) -> CachedDiff {
        CachedDiff {
            repository: "o/r".into(),
            from_version: "v1".into(),
            to_version: "v2".into(),
            from_commit: "a".into(),
            to_commit: "b".into(),
            comparison_status: "ahead".into(),
            files: Vec::new(),
            summary: DiffSummary::default(),
            total_changes: 0,
            generated_at: String::new(),
            created_at,
            zip: zip_name.map(|name| ZipArtifact {
                filename: name.into(),
                size: 1,
                sha256: String::new(),
                created_at,
                signature: None,
                public_key: None,
                manifest: None,
                manifest_signature: None,
            }),
        }
    }

    fn cache() -> Cache<String, CachedDiff> {
        let hour = Duration::from_secs(3600);
        Caches::new(hour, hour, hour).diffs
    }

    fn key(byte: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new([byte; 32])
    }

    #[tokio::test]
    async fn round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff_cache.json");
        let store = DiffCacheStore::new(path.clone(), Some(key(1)));
        let now = chrono::Utc::now().timestamp();

        let source = cache();
        source
            .insert(
                "fresh".into(),
                diff(now, Some("diff-0123456789abcdef0123456789abcdef.zip")),
            )
            .await;
        source
            .insert("bad-zip".into(), diff(now, Some("../../etc/passwd")))
            .await;

        // Nothing is written until the cache is marked dirty.
        store.flush(&source).await;
        assert!(!path.exists());
        store.mark_dirty();
        store.flush(&source).await;

        let restored = cache();
        DiffCacheStore::new(path, Some(key(1)))
            .load_into(&restored)
            .await;
        assert_eq!(restored.entry_count(), 2);
        assert!(restored.get("fresh").await.unwrap().zip.is_some());
        assert!(restored.get("bad-zip").await.unwrap().zip.is_none());
    }

    #[tokio::test]
    async fn tampered_or_foreign_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff_cache.json");
        let source = cache();
        source
            .insert("entry".into(), diff(chrono::Utc::now().timestamp(), None))
            .await;
        let store = DiffCacheStore::new(path.clone(), Some(key(1)));
        store.mark_dirty();
        store.flush(&source).await;

        // Another key (e.g. after a key rotation) or no key.
        for other in [Some(key(2)), None] {
            let restored = cache();
            DiffCacheStore::new(path.clone(), other)
                .load_into(&restored)
                .await;
            assert_eq!(restored.entry_count(), 0);
        }

        // A modified payload with the old MAC.
        let content = std::fs::read_to_string(&path).unwrap();
        std::fs::write(
            &path,
            content.replace("\"to_commit\":\"b\"", "\"to_commit\":\"c\""),
        )
        .unwrap();
        let restored = cache();
        DiffCacheStore::new(path.clone(), Some(key(1)))
            .load_into(&restored)
            .await;
        assert_eq!(restored.entry_count(), 0);

        // Files without a MAC line.
        std::fs::write(
            &path,
            r#"{"entries":{},"last_saved":"x","version":"1.1.1"}"#,
        )
        .unwrap();
        let restored = cache();
        DiffCacheStore::new(path, None).load_into(&restored).await;
        assert_eq!(restored.entry_count(), 0);
    }

    #[tokio::test]
    async fn expired_entries_are_dropped_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diff_cache.json");
        let now = chrono::Utc::now().timestamp();
        let snapshot = Snapshot {
            schema: SCHEMA_VERSION,
            saved_at: String::new(),
            entries: vec![
                ("old".to_string(), diff(now - 7200, None)),
                ("new".to_string(), diff(now - 60, None)),
            ],
        };
        let store = DiffCacheStore::new(path.clone(), None);
        let payload = serde_json::to_vec(&snapshot).unwrap();
        let mut file = store.mac(&payload).into_bytes();
        file.push(b'\n');
        file.extend_from_slice(&payload);
        std::fs::write(&path, file).unwrap();

        let restored = cache();
        store.load_into(&restored).await;
        assert!(restored.get("old").await.is_none());
        assert!(restored.get("new").await.is_some());
    }
}

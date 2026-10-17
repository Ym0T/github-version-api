//! Creation of (signed) ZIP archives for updates.
//!
//! An archive is only built when all of these hold:
//! - the repository is explicitly listed in `ALLOWED_REPOS` with a pinned id,
//! - both versions are tags, the diff was computed between their commits and
//!   `to` is a descendant of `from` (GitHub status `ahead`),
//! - `to` is the current latest version (no signed downgrades),
//! - every path is safe, no file turns into a directory or back, and the
//!   git tree of `to` lists only regular files within the size limits.
//!
//! These checks run before any build budget is spent; failures are cached
//! per diff. Every file is verified against its blob hash and any problem
//! fails the whole archive, so a partial update is never produced or signed.
//! Builds run in their own task (a client disconnect or request timeout
//! doesn't waste them) and are limited in concurrency, per hour, per client
//! and by a disk quota.

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, stream};
use moka::future::Cache;
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore, mpsc};
use tracing::info;

use crate::config::{RepoRef, ResolvedRepo, ZipConfig};
use crate::error::ApiError;
use crate::models::diff::{CachedDiff, ZipArtifact};
use crate::services::cleanup;
use crate::services::github::GitHubError;
use crate::services::signing::{ManifestFields, render_manifest};
use crate::services::version::latest_version;
use crate::services::zip_writer::{ArchiveEntry, WrittenArchive, write_archive};
use crate::state::AppState;
use crate::utils::budget::HourlyBudget;
use crate::utils::locks::KeyedLocks;
use crate::validation::{is_ancestor_path, is_sha1_hex, is_valid_zip_filename, safe_archive_path};

const SLOT_WAIT: Duration = Duration::from_secs(30);
/// Each download is held in memory until written; keep this small.
const PARALLEL_DOWNLOADS: usize = 2;
const PREFLIGHT_TTL: Duration = Duration::from_secs(600);
/// How long a failed build is remembered: long for failures that won't go
/// away by retrying, short for others.
const PERMANENT_FAILURE_TTL: Duration = Duration::from_secs(600);
const TRANSIENT_FAILURE_TTL: Duration = Duration::from_secs(60);

/// Limits concurrent and total archive builds and their disk usage.
pub struct ZipBuildGuard {
    slots: Arc<Semaphore>,
    locks: KeyedLocks,
    budget: HourlyBudget,
    client_budgets: Cache<IpAddr, Arc<HourlyBudget>>,
    max_per_client: u32,
    preflights: Cache<String, Arc<Preflight>>,
    failures: Cache<String, (ApiError, Instant)>,
    storage_check: Mutex<()>,
    reserved_bytes: Arc<AtomicU64>,
}

impl ZipBuildGuard {
    pub fn new(config: &ZipConfig) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(config.max_concurrent_builds)),
            locks: KeyedLocks::default(),
            budget: HourlyBudget::new(config.max_builds_per_hour),
            client_budgets: Cache::builder()
                .max_capacity(100_000)
                .time_to_idle(Duration::from_secs(3600))
                .build(),
            max_per_client: config.max_builds_per_client_per_hour,
            preflights: Cache::builder()
                .max_capacity(1_000)
                .time_to_live(PREFLIGHT_TTL)
                .build(),
            failures: Cache::builder()
                .max_capacity(10_000)
                .time_to_live(PERMANENT_FAILURE_TTL)
                .build(),
            storage_check: Mutex::new(()),
            reserved_bytes: Arc::new(AtomicU64::new(0)),
        }
    }

    async fn acquire_slot(&self) -> Result<OwnedSemaphorePermit, ApiError> {
        match tokio::time::timeout(SLOT_WAIT, Arc::clone(&self.slots).acquire_owned()).await {
            Ok(Ok(permit)) => Ok(permit),
            Ok(Err(closed)) => Err(ApiError::internal("ZIP build semaphore", closed)),
            Err(_) => Err(ApiError::unavailable(
                "server_busy",
                "Too many archives are being built right now, please retry shortly",
                Some(SLOT_WAIT.as_secs()),
            )),
        }
    }

    async fn client_budget(&self, client: IpAddr) -> Arc<HourlyBudget> {
        let limit = self.max_per_client;
        self.client_budgets
            .get_with(client, async move { Arc::new(HourlyBudget::new(limit)) })
            .await
    }

    async fn cached_failure(&self, key: &str) -> Option<ApiError> {
        self.failures
            .get(key)
            .await
            .filter(|(_, until)| *until > Instant::now())
            .map(|(err, _)| err)
    }

    async fn remember_failure(&self, key: &str, err: &ApiError) {
        let ttl = match err.code() {
            // Retrying right away can succeed.
            "github_rate_limited"
            | "server_busy"
            | "zip_build_limit_reached"
            | "zip_client_limit_reached"
            | "storage_full" => return,
            "unsafe_path"
            | "unsupported_change"
            | "unverifiable_file"
            | "missing_download_url"
            | "unsupported_file_type"
            | "repository_too_large"
            | "diff_too_large_for_zip"
            | "diff_zip_empty"
            | "content_verification_failed"
            | "github_inconsistent" => PERMANENT_FAILURE_TTL,
            _ => TRANSIENT_FAILURE_TTL,
        };
        self.failures
            .insert(key.to_string(), (err.clone(), Instant::now() + ttl))
            .await;
    }
}

/// Disk space reserved for a running build; released on drop.
struct StorageReservation {
    reserved: Arc<AtomicU64>,
    bytes: u64,
}

impl Drop for StorageReservation {
    fn drop(&mut self) {
        self.reserved.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

/// Rejects ZIP requests that are not allowed before any network I/O happens.
pub fn check_zip_allowed(state: &AppState, target: &ResolvedRepo) -> Result<(), ApiError> {
    if !target.trusted {
        return Err(ApiError::forbidden(
            "zip_not_allowed",
            "ZIP archives are only available for repositories explicitly listed in ALLOWED_REPOS",
        ));
    }
    if !state.config.zip_enabled() {
        return Err(ApiError::unavailable(
            "signing_unavailable",
            "ZIP archives are disabled because no signing key is configured",
            None,
        ));
    }
    if target.pinned_id.is_none() {
        return Err(ApiError::unavailable(
            "repository_not_pinned",
            "ZIP archives require the repository id to be pinned in ALLOWED_REPOS (owner/repo#id)",
            None,
        ));
    }
    Ok(())
}

/// Returns a usable archive for `diff`, building one if necessary.
pub async fn ensure_zip(
    state: &Arc<AppState>,
    target: &ResolvedRepo,
    client: IpAddr,
    key: &str,
    diff: &CachedDiff,
) -> Result<ZipArtifact, ApiError> {
    let guard = &state.zip_guard;
    if diff.comparison_status != "ahead" {
        return Err(ApiError::unprocessable(
            "versions_not_sequential",
            "ZIP archives require to_version to be a descendant of from_version",
        ));
    }
    let latest = latest_version(state, target).await?;
    if latest.version != diff.to_version {
        return Err(ApiError::unprocessable(
            "zip_target_not_latest",
            "ZIP archives are only created for updates to the latest version",
        ));
    }
    if let Some(artifact) = reuse(state, &target.repo, key, diff).await? {
        return Ok(artifact);
    }
    if let Some(err) = guard.cached_failure(key).await {
        return Err(err);
    }
    let preflight = match preflight(state, &target.repo, key, diff).await {
        Ok(preflight) => preflight,
        Err(err) => {
            guard.remember_failure(key, &err).await;
            return Err(err);
        }
    };

    // One build per diff; others wait here and then reuse the result.
    let held = guard.locks.get(key).lock_owned().await;
    if let Some(err) = guard.cached_failure(key).await {
        return Err(err);
    }
    let current = state
        .caches
        .diffs
        .get(key)
        .await
        .unwrap_or_else(|| diff.clone());
    if let Some(artifact) = reuse(state, &target.repo, key, &current).await? {
        return Ok(artifact);
    }

    // Budgets are only charged once a build really starts.
    let client_budget = guard.client_budget(client).await;
    guard.budget.check().map_err(build_limit_reached)?;
    client_budget.check().map_err(client_limit_reached)?;
    let slot = guard.acquire_slot().await?;
    let reservation = reserve_storage(state).await?;
    guard.budget.try_take().map_err(build_limit_reached)?;
    client_budget.try_take().map_err(client_limit_reached)?;

    let task = tokio::spawn({
        let state = Arc::clone(state);
        let repo = target.repo.clone();
        let key = key.to_string();
        async move {
            let _held = (held, slot, reservation);
            match build(&state, &repo, &current, &preflight).await {
                Ok(artifact) => {
                    let mut updated = current;
                    updated.zip = Some(artifact.clone());
                    state.caches.diffs.insert(key, updated).await;
                    state.diff_store.mark_dirty();
                    Ok(artifact)
                }
                Err(err) => {
                    state.zip_guard.remember_failure(&key, &err).await;
                    Err(err)
                }
            }
        }
    });
    task.await
        .map_err(|e| ApiError::internal("ZIP build task failed", e))?
}

fn build_limit_reached(retry_after: u64) -> ApiError {
    ApiError::unavailable(
        "zip_build_limit_reached",
        "The hourly limit for new archives is reached, please retry later",
        Some(retry_after),
    )
}

fn client_limit_reached(retry_after: u64) -> ApiError {
    ApiError::rate_limited(retry_after).with_code("zip_client_limit_reached")
}

/// An existing archive that may still be handed out, signed according to
/// the current configuration (re-signed after a key rotation).
async fn reuse(
    state: &AppState,
    repo: &RepoRef,
    key: &str,
    diff: &CachedDiff,
) -> Result<Option<ZipArtifact>, ApiError> {
    let Some(artifact) = &diff.zip else {
        return Ok(None);
    };
    let age = chrono::Utc::now()
        .timestamp()
        .saturating_sub(artifact.created_at);
    let fresh = u64::try_from(age).is_ok_and(|age| age < state.config.zip.reuse_window().as_secs());
    if !fresh || !is_valid_zip_filename(&artifact.filename) {
        return Ok(None);
    }
    let path = state.config.diffs_dir.join(&artifact.filename);
    let is_file = tokio::fs::symlink_metadata(&path)
        .await
        .is_ok_and(|meta| meta.file_type().is_file());
    if !is_file {
        return Ok(None);
    }

    let signed = sign_artifact(state, repo, diff, artifact)?;
    if signed.manifest_signature != artifact.manifest_signature
        || signed.signature != artifact.signature
    {
        let mut updated = diff.clone();
        updated.zip = Some(signed.clone());
        state.caches.diffs.insert(key.to_string(), updated).await;
        state.diff_store.mark_dirty();
    }
    Ok(Some(signed))
}

/// Returns `artifact` with the signatures the current configuration asks for.
fn sign_artifact(
    state: &AppState,
    repo: &RepoRef,
    diff: &CachedDiff,
    artifact: &ZipArtifact,
) -> Result<ZipArtifact, ApiError> {
    let legacy = state.config.zip.legacy_signatures;
    let Some(signer) = &state.signer else {
        return Ok(ZipArtifact {
            signature: None,
            public_key: None,
            manifest: None,
            manifest_signature: None,
            ..artifact.clone()
        });
    };
    let up_to_date = artifact.public_key.as_deref() == Some(signer.public_key_b64())
        && artifact.manifest.is_some()
        && artifact.manifest_signature.is_some()
        && artifact.signature.is_some() == legacy;
    if up_to_date {
        return Ok(artifact.clone());
    }

    let plan = plan(diff)?;
    let manifest = render_manifest(&ManifestFields {
        repository: &repo.to_string(),
        from: &diff.from_version,
        to: &diff.to_version,
        from_commit: &diff.from_commit,
        to_commit: &diff.to_commit,
        sha256_hex: &artifact.sha256,
        size: artifact.size,
        created_at: artifact.created_at,
        delete: &plan.delete,
    });
    let signature = if legacy {
        let digest: [u8; 32] = hex::decode(&artifact.sha256)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or_else(|| {
                ApiError::internal("re-signing archive", "stored checksum is invalid")
            })?;
        Some(signer.sign(&digest))
    } else {
        None
    };
    Ok(ZipArtifact {
        signature,
        public_key: Some(signer.public_key_b64().to_string()),
        manifest_signature: Some(signer.sign(manifest.as_bytes())),
        manifest: Some(manifest),
        ..artifact.clone()
    })
}

async fn reserve_storage(state: &AppState) -> Result<StorageReservation, ApiError> {
    let guard = &state.zip_guard;
    let zip = &state.config.zip;
    let dir = &state.config.diffs_dir;
    let _checking = guard.storage_check.lock().await;
    let fits = |used: u64| {
        used.saturating_add(guard.reserved_bytes.load(Ordering::Acquire))
            .saturating_add(zip.max_total_bytes)
            <= zip.storage_quota_bytes
    };

    let measure = || async {
        cleanup::storage_usage(dir)
            .await
            .map_err(|e| ApiError::internal("cannot measure archive storage", e))
    };
    if !fits(measure().await?) {
        cleanup::run_once(dir, zip.retention)
            .await
            .map_err(|e| ApiError::internal("archive cleanup failed", e))?;
        if !fits(measure().await?) {
            return Err(ApiError::unavailable(
                "storage_full",
                "The archive storage quota is exhausted, please retry later",
                Some(600),
            ));
        }
    }
    guard
        .reserved_bytes
        .fetch_add(zip.max_total_bytes, Ordering::AcqRel);
    Ok(StorageReservation {
        reserved: Arc::clone(&guard.reserved_bytes),
        bytes: zip.max_total_bytes,
    })
}

/// What goes into an archive: paths validated, nothing downloaded yet.
#[derive(Debug)]
struct Plan {
    files: Vec<PlannedFile>,
    /// Sorted, unique paths the client must delete.
    delete: Vec<String>,
}

#[derive(Debug, Clone)]
struct PlannedFile {
    path: String,
    sha: String,
    url: String,
}

/// A plan checked against the git tree of `to`.
#[derive(Debug)]
pub struct Preflight {
    files: Vec<CheckedFile>,
}

#[derive(Debug, Clone)]
struct CheckedFile {
    file: PlannedFile,
    mode: u32,
}

fn unsafe_path() -> ApiError {
    ApiError::unprocessable(
        "unsafe_path",
        "The diff contains a file path that cannot be packaged safely",
    )
}

fn plan(diff: &CachedDiff) -> Result<Plan, ApiError> {
    let mut files = Vec::new();
    let mut delete = Vec::new();
    for file in &diff.files {
        if file.is_removed() {
            delete.push(
                safe_archive_path(&file.filename)
                    .ok_or_else(unsafe_path)?
                    .to_string(),
            );
            continue;
        }
        let path = safe_archive_path(&file.filename).ok_or_else(unsafe_path)?;
        if file.status == "renamed"
            && let Some(previous) = &file.previous_filename
        {
            delete.push(
                safe_archive_path(previous)
                    .ok_or_else(unsafe_path)?
                    .to_string(),
            );
        }
        let sha = file
            .sha
            .as_deref()
            .filter(|s| is_sha1_hex(s))
            .ok_or_else(|| {
                ApiError::unprocessable(
                    "unverifiable_file",
                    "GitHub did not provide a content hash for a changed file",
                )
            })?;
        let url = file.raw_url.as_deref().ok_or_else(|| {
            ApiError::unprocessable(
                "missing_download_url",
                "GitHub did not provide a download URL for a changed file (for example a submodule)",
            )
        })?;
        files.push(PlannedFile {
            path: path.to_string(),
            sha: sha.to_ascii_lowercase(),
            url: url.to_string(),
        });
    }
    if files.is_empty() {
        return Err(ApiError::unprocessable(
            "diff_zip_empty",
            "The diff only removes files, there is nothing to put into a ZIP archive",
        ));
    }
    // A path that is deleted and written again (e.g. swapped renames) must
    // not be deleted.
    delete.retain(|path| !files.iter().any(|f| &f.path == path));
    delete.sort();
    delete.dedup();

    // Files becoming directories (or the reverse) can't be applied as a
    // simple "delete, then copy" update.
    let conflict = delete.iter().any(|deleted| {
        files
            .iter()
            .any(|f| is_ancestor_path(deleted, &f.path) || is_ancestor_path(&f.path, deleted))
    });
    if conflict {
        return Err(ApiError::unprocessable(
            "unsupported_change",
            "The update turns a file into a directory or the other way round; use a full update",
        ));
    }
    Ok(Plan { files, delete })
}

/// Validates the plan and checks the files against the git tree of `to`
/// (regular files only, size limits). Cached per diff.
async fn preflight(
    state: &AppState,
    repo: &RepoRef,
    key: &str,
    diff: &CachedDiff,
) -> Result<Arc<Preflight>, ApiError> {
    state
        .zip_guard
        .preflights
        .entry(key.to_string())
        .or_try_insert_with(check_against_tree(state, repo, diff))
        .await
        .map(moka::Entry::into_value)
        .map_err(Arc::unwrap_or_clone)
}

async fn check_against_tree(
    state: &AppState,
    repo: &RepoRef,
    diff: &CachedDiff,
) -> Result<Arc<Preflight>, ApiError> {
    let limits = &state.config.zip;
    let plan = plan(diff)?;
    let too_large = || {
        ApiError::unprocessable(
            "repository_too_large",
            "The repository tree is too large to verify the file types of this update",
        )
    };
    let inconsistent = || {
        ApiError::bad_gateway(
            "github_inconsistent",
            "GitHub returned inconsistent data for this update",
        )
    };
    let file_too_large = || {
        ApiError::unprocessable(
            "diff_too_large_for_zip",
            "The changed files exceed the size limits for ZIP archives",
        )
    };

    let tree = state
        .github
        .tree(repo, &diff.to_commit)
        .await
        .map_err(|err| match err {
            GitHubError::TooLarge => too_large(),
            other => other.into(),
        })?;
    if tree.truncated {
        return Err(too_large());
    }
    let entries: HashMap<&str, _> = tree.tree.iter().map(|e| (e.path.as_str(), e)).collect();

    let mut total = 0u64;
    let mut files = Vec::with_capacity(plan.files.len());
    for file in plan.files {
        let entry = entries.get(file.path.as_str()).ok_or_else(inconsistent)?;
        let mode = match entry.mode.as_str() {
            "100644" => 0o644,
            "100755" => 0o755,
            _ => {
                return Err(ApiError::unprocessable(
                    "unsupported_file_type",
                    "The update contains a symlink, submodule or other special file that cannot be packaged",
                ));
            }
        };
        if !entry.sha.eq_ignore_ascii_case(&file.sha) {
            return Err(inconsistent());
        }
        let size = entry.size.ok_or_else(inconsistent)?;
        total = total.saturating_add(size);
        if size > limits.max_file_bytes || total > limits.max_total_bytes {
            return Err(file_too_large());
        }
        files.push(CheckedFile { file, mode });
    }
    Ok(Arc::new(Preflight { files }))
}

enum FeedError {
    Api(ApiError),
    WriterStopped,
}

/// Downloads and verifies all files and writes the archive.
async fn build(
    state: &AppState,
    repo: &RepoRef,
    diff: &CachedDiff,
    preflight: &Preflight,
) -> Result<ZipArtifact, ApiError> {
    let filename = format!("diff-{}.zip", uuid::Uuid::new_v4().simple());
    let final_path = state.config.diffs_dir.join(&filename);
    let part_path = state.config.diffs_dir.join(format!("{filename}.part"));
    let written = write_files(state, preflight.files.clone(), &part_path).await?;

    if let Err(err) = tokio::fs::rename(&part_path, &final_path).await {
        let _ = tokio::fs::remove_file(&part_path).await;
        return Err(ApiError::internal("cannot finalize ZIP archive", err));
    }
    let dir = state.config.diffs_dir.clone();
    let _ = tokio::task::spawn_blocking(move || crate::utils::fs::sync_dir(&dir)).await;

    let unsigned = ZipArtifact {
        filename,
        size: written.size,
        sha256: hex::encode(written.sha256),
        created_at: chrono::Utc::now().timestamp(),
        signature: None,
        public_key: None,
        manifest: None,
        manifest_signature: None,
    };
    let artifact = sign_artifact(state, repo, diff, &unsigned)?;
    info!(
        repository = %repo,
        from = %diff.from_version,
        to = %diff.to_version,
        file = %artifact.filename,
        size = artifact.size,
        signed = artifact.manifest_signature.is_some(),
        "created diff archive"
    );
    Ok(artifact)
}

/// Downloads the files (verified against their blob hashes) and writes them
/// into `part_path` on a blocking thread.
async fn write_files(
    state: &AppState,
    files: Vec<CheckedFile>,
    part_path: &Path,
) -> Result<WrittenArchive, ApiError> {
    let limits = &state.config.zip;

    // Capacity 1 keeps only a few files in memory at once.
    let (tx, rx) = mpsc::channel(1);
    let writer = {
        let part_path = part_path.to_path_buf();
        tokio::task::spawn_blocking(move || write_archive(&part_path, rx))
    };

    // `tx` is moved into the future and dropped when it finishes; if a
    // download fails before `Finish`, the writer aborts and deletes the file.
    let feed = async move {
        let github = &state.github;
        let mut downloads = stream::iter(files)
            .map(|checked| async move {
                let data = github
                    .download_blob(&checked.file.url, &checked.file.sha, limits.max_file_bytes)
                    .await;
                (checked, data)
            })
            .buffered(PARALLEL_DOWNLOADS);
        let mut total = 0u64;
        while let Some((checked, data)) = downloads.next().await {
            let data = data.map_err(|err| {
                FeedError::Api(match err {
                    GitHubError::TooLarge => ApiError::unprocessable(
                        "diff_too_large_for_zip",
                        "A changed file exceeds the per-file size limit for ZIP archives",
                    ),
                    other => ApiError::from(other),
                })
            })?;
            total += data.len() as u64;
            if total > limits.max_total_bytes {
                return Err(FeedError::Api(ApiError::unprocessable(
                    "diff_too_large_for_zip",
                    "The changed files exceed the total size limit for ZIP archives",
                )));
            }
            let entry = ArchiveEntry::File {
                path: checked.file.path,
                data,
                mode: checked.mode,
            };
            tx.send(entry).await.map_err(|_| FeedError::WriterStopped)?;
        }
        tx.send(ArchiveEntry::Finish)
            .await
            .map_err(|_| FeedError::WriterStopped)
    };

    let feed_result = feed.await;
    let writer_result = writer.await;
    match (feed_result, writer_result) {
        (Ok(()), Ok(Ok(written))) => Ok(written),
        (Err(FeedError::Api(err)), _) => Err(err),
        (_, Ok(Err(err))) => Err(ApiError::internal(
            "writing ZIP archive failed",
            format!("{err:#}"),
        )),
        (_, Err(join_error)) => {
            let _ = tokio::fs::remove_file(part_path).await;
            Err(ApiError::internal("ZIP writer task failed", join_error))
        }
        (Err(FeedError::WriterStopped), Ok(Ok(_))) => Err(ApiError::internal(
            "ZIP build",
            "writer finished before all files were sent",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::diff::{DiffSummary, FileChange};

    fn diff_with(files: Vec<FileChange>) -> CachedDiff {
        CachedDiff {
            repository: "o/r".into(),
            from_version: "v1".into(),
            to_version: "v2".into(),
            from_commit: "a".repeat(40),
            to_commit: "b".repeat(40),
            comparison_status: "ahead".into(),
            summary: DiffSummary::from_files(&files),
            total_changes: 0,
            files,
            generated_at: String::new(),
            created_at: 0,
            zip: None,
        }
    }

    fn file(name: &str, status: &str, sha: Option<&str>, url: Option<&str>) -> FileChange {
        FileChange {
            filename: name.into(),
            status: status.into(),
            additions: 1,
            deletions: 0,
            changes: 1,
            sha: sha.map(Into::into),
            patch_url: None,
            raw_url: url.map(Into::into),
            previous_filename: None,
        }
    }

    const SHA: &str = "ce013625030ba8dba906f756967f9e9ca394464a";
    const URL: &str = "https://github.com/o/r/raw/abc/file";

    #[test]
    fn plan_collects_files_and_deletions() {
        let mut renamed = file("new.txt", "renamed", Some(SHA), Some(URL));
        renamed.previous_filename = Some("old/name.txt".into());
        let mut swapped = file("a.txt", "renamed", Some(SHA), Some(URL));
        swapped.previous_filename = Some("src/a.rs".into());
        let plan = plan(&diff_with(vec![
            file("src/a.rs", "modified", Some(SHA), Some(URL)),
            file("removed.txt", "removed", None, None),
            renamed,
            swapped,
        ]))
        .unwrap();
        assert_eq!(plan.files.len(), 3);
        // `src/a.rs` is still written, so it must not be deleted.
        assert_eq!(plan.delete, ["old/name.txt", "removed.txt"]);
    }

    #[test]
    fn plan_rejects_unsafe_or_unverifiable_files() {
        let mut bad_rename = file("ok.txt", "renamed", Some(SHA), Some(URL));
        bad_rename.previous_filename = Some("../../etc/passwd".into());
        let cases = [
            (
                file("../evil", "added", Some(SHA), Some(URL)),
                "unsafe_path",
            ),
            (
                file("....\\/evil", "added", Some(SHA), Some(URL)),
                "unsafe_path",
            ),
            (
                file(".git/hooks/post-merge", "added", Some(SHA), Some(URL)),
                "unsafe_path",
            ),
            (
                file("a\u{2028}sha256=x", "added", Some(SHA), Some(URL)),
                "unsafe_path",
            ),
            (
                file("../../home/container/start.sh", "removed", None, None),
                "unsafe_path",
            ),
            (bad_rename, "unsafe_path"),
            (file("a", "added", None, Some(URL)), "unverifiable_file"),
            (
                file("a", "added", Some("xyz"), Some(URL)),
                "unverifiable_file",
            ),
            (file("a", "added", Some(SHA), None), "missing_download_url"),
        ];
        for (bad, code) in cases {
            let err = plan(&diff_with(vec![
                file("ok", "added", Some(SHA), Some(URL)),
                bad,
            ]))
            .unwrap_err();
            assert_eq!(err.code(), code);
        }

        let only_removed = diff_with(vec![file("old.rs", "removed", None, None)]);
        assert_eq!(plan(&only_removed).unwrap_err().code(), "diff_zip_empty");
    }

    #[test]
    fn plan_rejects_file_directory_transitions() {
        for files in [
            vec![
                file("conf", "removed", None, None),
                file("conf/default.conf", "added", Some(SHA), Some(URL)),
            ],
            vec![
                file("conf/default.conf", "removed", None, None),
                file("conf", "added", Some(SHA), Some(URL)),
            ],
        ] {
            assert_eq!(
                plan(&diff_with(files)).unwrap_err().code(),
                "unsupported_change"
            );
        }
    }
}

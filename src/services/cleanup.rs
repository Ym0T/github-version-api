//! Removes expired diff archives and leftovers of interrupted builds.
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use tokio::fs;
use tokio::task::JoinHandle;
use tracing::{info, warn};

const INTERVAL: Duration = Duration::from_secs(15 * 60);
/// Partial files older than this belong to builds that were interrupted.
const PARTIAL_MAX_AGE: Duration = Duration::from_secs(3600);

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CleanupReport {
    pub removed: usize,
    pub freed_bytes: u64,
}

/// Runs the cleanup at startup and then every 15 minutes.
pub fn spawn(dir: PathBuf, retention: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(INTERVAL);
        loop {
            interval.tick().await;
            match run_once(&dir, retention).await {
                Ok(report) if report.removed > 0 => info!(
                    removed = report.removed,
                    freed_bytes = report.freed_bytes,
                    "removed expired diff archives"
                ),
                Ok(_) => {}
                Err(e) => warn!("diff archive cleanup failed: {e}"),
            }
        }
    })
}

pub async fn run_once(dir: &Path, retention: Duration) -> io::Result<CleanupReport> {
    let mut report = CleanupReport::default();
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(report),
        Err(e) => return Err(e),
    };
    let now = SystemTime::now();

    while let Some(entry) = entries.next_entry().await? {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let max_age = if is_archive(&name) {
            retention
        } else if is_partial(&name) {
            PARTIAL_MAX_AGE
        } else {
            continue;
        };
        // Never follow symlinks.
        let Ok(meta) = fs::symlink_metadata(entry.path()).await else {
            continue;
        };
        if !meta.file_type().is_file() || age(&meta, now) < max_age {
            continue;
        }
        match fs::remove_file(entry.path()).await {
            Ok(()) => {
                report.removed += 1;
                report.freed_bytes += meta.len();
            }
            Err(e) => warn!("cannot remove {}: {e}", entry.path().display()),
        }
    }
    Ok(report)
}

/// Total size of all regular files in `dir`.
pub async fn storage_usage(dir: &Path) -> io::Result<u64> {
    let mut entries = match fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut total = 0u64;
    while let Some(entry) = entries.next_entry().await? {
        if let Ok(meta) = fs::symlink_metadata(entry.path()).await
            && meta.file_type().is_file()
        {
            total = total.saturating_add(meta.len());
        }
    }
    Ok(total)
}

/// Age of a file based on its modification time; unknown times count as new.
pub fn age(meta: &std::fs::Metadata, now: SystemTime) -> Duration {
    meta.modified()
        .ok()
        .and_then(|modified| now.duration_since(modified).ok())
        .unwrap_or_default()
}

/// Anything named like an archive, so stray `diff-*.zip` files with other
/// names are removed as well.
fn is_archive(name: &str) -> bool {
    name.starts_with("diff-") && name.ends_with(".zip")
}

fn is_partial(name: &str) -> bool {
    (name.starts_with("diff-") && name.ends_with(".zip.part"))
        || (name.starts_with("test-") && name.ends_with(".tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(dir: &Path, name: &str, age: Duration) {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).unwrap();
        std::io::Write::write_all(&mut &file, b"12345").unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
    }

    #[tokio::test]
    async fn removes_only_expired_service_files() {
        let dir = tempfile::tempdir().unwrap();
        let day = Duration::from_secs(24 * 3600);
        create(
            dir.path(),
            "diff-0123456789abcdef0123456789abcdef.zip",
            day * 2,
        );
        create(dir.path(), "diff-Ym0T-repo-v1-v2-abcd1234.zip", day * 2);
        create(
            dir.path(),
            "diff-fedcba9876543210fedcba9876543210.zip",
            Duration::from_secs(60),
        );
        create(dir.path(), "diff-x.zip.part", Duration::from_secs(2 * 3600));
        create(dir.path(), "diff-y.zip.part", Duration::from_secs(60));
        create(dir.path(), "unrelated.txt", day * 10);

        assert_eq!(storage_usage(dir.path()).await.unwrap(), 30);
        let report = run_once(dir.path(), day).await.unwrap();
        assert_eq!(
            report,
            CleanupReport {
                removed: 3,
                freed_bytes: 15
            }
        );

        let mut left: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(
            left,
            [
                "diff-fedcba9876543210fedcba9876543210.zip",
                "diff-y.zip.part",
                "unrelated.txt"
            ]
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn does_not_follow_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        create(
            outside.path(),
            "victim.zip",
            Duration::from_secs(10 * 24 * 3600),
        );
        std::os::unix::fs::symlink(
            outside.path().join("victim.zip"),
            dir.path().join("diff-link.zip"),
        )
        .unwrap();

        run_once(dir.path(), Duration::from_secs(1)).await.unwrap();
        assert!(outside.path().join("victim.zip").exists());
    }
}

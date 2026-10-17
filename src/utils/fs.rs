//! File system helpers. Files are created with owner-only permissions and
//! replaced atomically, so readers never see partial content.
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Creates `path` (and parents). A directory created here is restricted to
/// the owner; existing directories are left as they are.
pub async fn create_private_dir_all(path: &Path) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        if path.is_dir() {
            return Ok(());
        }
        std::fs::create_dir_all(&path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
        }
        Ok(())
    })
    .await
    .map_err(io::Error::other)?
}

/// Creates a new file that must not exist yet (refuses to follow or replace
/// symlinks) and is only accessible by the owner.
pub fn create_new_private_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).read(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Opens a file for reading without following a symlink at the final path
/// component.
pub async fn open_nofollow(path: &Path) -> io::Result<tokio::fs::File> {
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW);
    options.open(path).await
}

/// Atomically replaces `path` with `data` (temp file, fsync, rename, fsync
/// of the directory).
pub async fn atomic_write(path: &Path, data: Vec<u8>) -> io::Result<()> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || atomic_write_blocking(&path, &data))
        .await
        .map_err(io::Error::other)?
}

fn atomic_write_blocking(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    if let Some(parent) = parent {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = temp_path(path);
    let result = (|| {
        let mut file = create_new_private_file(&tmp)?;
        file.write_all(data)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result?;
    sync_dir(parent.unwrap_or(Path::new(".")))
}

/// Makes a rename durable. A no-op where directories can't be opened.
pub fn sync_dir(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = dir;
    Ok(())
}

fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.tmp", uuid::Uuid::new_v4().simple()))
}

/// Reads a file if it is not larger than `max_bytes`.
pub async fn read_limited(path: &Path, max_bytes: u64) -> io::Result<Vec<u8>> {
    let meta = tokio::fs::metadata(path).await?;
    if meta.len() > max_bytes {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{} is larger than {max_bytes} bytes", path.display()),
        ));
    }
    tokio::fs::read(path).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn atomic_write_replaces_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested/state.json");
        atomic_write(&path, b"one".to_vec()).await.unwrap();
        atomic_write(&path, b"two".to_vec()).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"two");
        // No temp files are left behind.
        assert_eq!(
            std::fs::read_dir(path.parent().unwrap()).unwrap().count(),
            1
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn create_new_refuses_existing_files_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("file");
        create_new_private_file(&path).unwrap();
        assert!(create_new_private_file(&path).is_err());

        #[cfg(unix)]
        {
            let target = dir.path().join("target");
            let link = dir.path().join("link");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert!(create_new_private_file(&link).is_err());
            assert!(!target.exists());
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn open_nofollow_rejects_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::write(&target, b"x").unwrap();
        let link = dir.path().join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(open_nofollow(&target).await.is_ok());
        assert!(open_nofollow(&link).await.is_err());
    }

    #[tokio::test]
    async fn read_limited_rejects_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big");
        std::fs::write(&path, vec![0u8; 100]).unwrap();
        assert!(read_limited(&path, 99).await.is_err());
        assert_eq!(read_limited(&path, 100).await.unwrap().len(), 100);
    }
}

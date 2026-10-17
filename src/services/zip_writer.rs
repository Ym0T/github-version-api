//! Writes ZIP archives on a blocking thread, fed through a channel.
use std::io::{BufWriter, Read, Write};
use std::path::Path;

use anyhow::bail;
use sha2::{Digest, Sha256};
use tokio::sync::mpsc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

pub enum ArchiveEntry {
    File {
        path: String,
        data: Vec<u8>,
        mode: u32,
    },
    Finish,
}

pub struct WrittenArchive {
    pub size: u64,
    pub sha256: [u8; 32],
}

/// Writes all entries received before `Finish` into a new file at
/// `part_path`. If the sender goes away without `Finish`, or anything fails,
/// the file is removed.
pub fn write_archive(
    part_path: &Path,
    mut rx: mpsc::Receiver<ArchiveEntry>,
) -> anyhow::Result<WrittenArchive> {
    let result = write_archive_inner(part_path, &mut rx);
    if result.is_err() {
        let _ = std::fs::remove_file(part_path);
    }
    result
}

fn write_archive_inner(
    part_path: &Path,
    rx: &mut mpsc::Receiver<ArchiveEntry>,
) -> anyhow::Result<WrittenArchive> {
    let file = crate::utils::fs::create_new_private_file(part_path)?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    loop {
        match rx.blocking_recv() {
            Some(ArchiveEntry::File { path, data, mode }) => {
                zip.start_file(path, options.unix_permissions(mode))?;
                zip.write_all(&data)?;
            }
            Some(ArchiveEntry::Finish) => break,
            None => bail!("archive input ended before completion"),
        }
    }
    let file = zip
        .finish()?
        .into_inner()
        .map_err(std::io::IntoInnerError::into_error)?;
    file.sync_all()?;
    drop(file);
    hash_file(part_path)
}

fn hash_file(path: &Path) -> anyhow::Result<WrittenArchive> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 * 1024];
    let mut size = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        size += read as u64;
    }
    Ok(WrittenArchive {
        size,
        sha256: hasher.finalize().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_valid_archive_with_modes() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("diff-test.zip.part");
        let (tx, rx) = mpsc::channel(4);
        tx.blocking_send(ArchiveEntry::File {
            path: "src/a.txt".into(),
            data: b"hello\n".to_vec(),
            mode: 0o644,
        })
        .unwrap();
        tx.blocking_send(ArchiveEntry::File {
            path: "start.sh".into(),
            data: b"#!/bin/sh\n".to_vec(),
            mode: 0o755,
        })
        .unwrap();
        tx.blocking_send(ArchiveEntry::Finish).unwrap();
        drop(tx);

        let written = write_archive(&part, rx).unwrap();
        let bytes = std::fs::read(&part).unwrap();
        assert_eq!(written.size, bytes.len() as u64);
        assert_eq!(written.sha256, <[u8; 32]>::from(Sha256::digest(&bytes)));

        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
        assert_eq!(archive.len(), 2, "no generated extra files");
        let mut content = String::new();
        archive
            .by_name("src/a.txt")
            .unwrap()
            .read_to_string(&mut content)
            .unwrap();
        assert_eq!(content, "hello\n");
        assert_eq!(
            archive.by_name("start.sh").unwrap().unix_mode().unwrap() & 0o777,
            0o755
        );
    }

    #[test]
    fn aborted_archive_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let part = dir.path().join("diff-test.zip.part");
        let (tx, rx) = mpsc::channel(4);
        tx.blocking_send(ArchiveEntry::File {
            path: "a.txt".into(),
            data: b"partial".to_vec(),
            mode: 0o644,
        })
        .unwrap();
        drop(tx); // no Finish: the producer failed

        assert!(write_archive(&part, rx).is_err());
        assert!(!part.exists());
    }
}

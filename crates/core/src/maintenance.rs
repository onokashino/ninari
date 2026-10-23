//! Keeping the app's files in check: how much the caches take and emptying them.

use std::path::Path;

/// Bytes taken by the files under `dir`, subfolders included; 0 when it does not exist.
pub fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Deletes everything inside `dir` and keeps the folder; returns the bytes freed.
/// A file that is busy stays and is not counted.
pub fn clear_dir(dir: &Path) -> std::io::Result<u64> {
    let mut freed = 0;
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(0);
    };
    for e in entries.flatten() {
        let path = e.path();
        let size = match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&path),
            Ok(m) => m.len(),
            Err(_) => 0,
        };
        let removed = if path.is_dir() {
            std::fs::remove_dir_all(&path)
        } else {
            std::fs::remove_file(&path)
        };
        if removed.is_ok() {
            freed += size;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_is_measured_and_emptied_but_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), vec![0u8; 1000]).unwrap();
        std::fs::create_dir(dir.path().join("people")).unwrap();
        std::fs::write(dir.path().join("people").join("b.jpg"), vec![0u8; 500]).unwrap();
        assert_eq!(dir_size(dir.path()), 1500);
        assert_eq!(clear_dir(dir.path()).unwrap(), 1500);
        assert!(dir.path().exists());
        assert_eq!(dir_size(dir.path()), 0);
        assert_eq!(dir_size(&dir.path().join("missing")), 0);
    }
}

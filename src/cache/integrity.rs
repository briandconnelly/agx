//! SHA-256 integrity hashing and verification.

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Compute the SHA-256 hash of a file, returning it as a hex string.
pub fn hash_file(path: &Path) -> Result<String> {
    let content =
        std::fs::read(path).with_context(|| format!("reading {} for hashing", path.display()))?;
    Ok(hash_bytes(&content))
}

/// Compute the SHA-256 hash of a byte slice.
pub fn hash_bytes(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    format!("sha256:{:x}", hasher.finalize())
}

/// Compute the SHA-256 hash of a directory (by hashing all files in sorted order).
pub fn hash_dir(path: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut entries: Vec<_> = walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .collect();
    entries.sort_by_key(|e| e.path().to_path_buf());

    for entry in entries {
        // Hash the relative path + file content for determinism
        let relative = entry.path().strip_prefix(path).unwrap_or(entry.path());
        hasher.update(relative.to_string_lossy().as_bytes());
        let content = std::fs::read(entry.path())?;
        hasher.update(&content);
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

/// Verify that a file matches an expected integrity hash.
pub fn verify_file(path: &Path, expected: &str) -> Result<bool> {
    let actual = hash_file(path)?;
    Ok(actual == expected)
}

/// Verify that a directory matches an expected integrity hash.
pub fn verify_dir(path: &Path, expected: &str) -> Result<bool> {
    let actual = hash_dir(path)?;
    Ok(actual == expected)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hash_bytes_deterministic() {
        let h1 = hash_bytes(b"hello world");
        let h2 = hash_bytes(b"hello world");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn hash_bytes_different_input() {
        let h1 = hash_bytes(b"hello");
        let h2 = hash_bytes(b"world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_file_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        std::fs::write(&file_path, b"test content for hashing").unwrap();

        let hash = hash_file(&file_path).unwrap();
        assert!(hash.starts_with("sha256:"));

        // Verification should succeed with correct hash
        assert!(verify_file(&file_path, &hash).unwrap());

        // Verification should fail with wrong hash
        assert!(!verify_file(&file_path, "sha256:0000000000").unwrap());
    }

    #[test]
    fn hash_file_verify_detects_corruption() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("data.bin");
        std::fs::write(&file_path, b"original content").unwrap();

        let hash = hash_file(&file_path).unwrap();
        assert!(verify_file(&file_path, &hash).unwrap());

        // Corrupt the file
        std::fs::write(&file_path, b"corrupted content").unwrap();
        assert!(!verify_file(&file_path, &hash).unwrap());
    }

    #[test]
    fn hash_dir_deterministic() {
        let dir = tempfile::tempdir().unwrap();

        // Create multiple files
        let sub = dir.path().join("sub");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(dir.path().join("a.txt"), b"file a").unwrap();
        std::fs::write(dir.path().join("b.txt"), b"file b").unwrap();
        std::fs::write(sub.join("c.txt"), b"file c").unwrap();

        let h1 = hash_dir(dir.path()).unwrap();
        let h2 = hash_dir(dir.path()).unwrap();
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn hash_dir_and_verify() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("x.txt"), b"content x").unwrap();
        std::fs::write(dir.path().join("y.txt"), b"content y").unwrap();

        let hash = hash_dir(dir.path()).unwrap();
        assert!(verify_dir(dir.path(), &hash).unwrap());

        // Modify a file and verify it no longer matches
        std::fs::write(dir.path().join("x.txt"), b"changed").unwrap();
        assert!(!verify_dir(dir.path(), &hash).unwrap());
    }

    #[test]
    fn hash_dir_different_content_different_hash() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        std::fs::write(dir1.path().join("file.txt"), b"content A").unwrap();
        std::fs::write(dir2.path().join("file.txt"), b"content B").unwrap();

        let h1 = hash_dir(dir1.path()).unwrap();
        let h2 = hash_dir(dir2.path()).unwrap();
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_dir_filename_matters() {
        // Two dirs with same content but different filenames should differ
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        std::fs::write(dir1.path().join("alpha.txt"), b"data").unwrap();
        std::fs::write(dir2.path().join("beta.txt"), b"data").unwrap();

        let h1 = hash_dir(dir1.path()).unwrap();
        let h2 = hash_dir(dir2.path()).unwrap();
        assert_ne!(h1, h2);
    }
}

//! Vault size calculation and human-readable formatting in GB or MB.

use std::path::Path;

/// Computes the total byte size of all files in `vault_dir` recursively.
///
/// Skips `.git` directories to accurately measure user vault content.
pub fn compute_vault_size(vault_dir: &Path) -> u64 {
    let mut total: u64 = 0;
    let mut stack = vec![vault_dir.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name == ".git" {
                    continue;
                }
            }

            match entry.metadata() {
                Ok(meta) => {
                    if meta.is_dir() {
                        stack.push(path);
                    } else if meta.is_file() {
                        total = total.saturating_add(meta.len());
                    }
                }
                Err(_) => continue,
            }
        }
    }

    total
}

/// Formats a byte size into a human-readable string in GB or MB.
pub fn format_vault_size(bytes: u64) -> String {
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        let mb = b / MB;
        if mb >= 1.0 {
            format!("{:.1} MB", mb)
        } else if mb >= 0.005 {
            format!("{:.2} MB", mb)
        } else if bytes > 0 {
            "< 0.01 MB".to_string()
        } else {
            "0 MB".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_vault_size() {
        assert_eq!(format_vault_size(0), "0 MB");
        assert_eq!(format_vault_size(500), "< 0.01 MB");
        assert_eq!(format_vault_size(10 * 1024), "0.01 MB");
        assert_eq!(format_vault_size(250 * 1024), "0.24 MB");
        assert_eq!(format_vault_size(1024 * 1024), "1.0 MB");
        assert_eq!(format_vault_size(15 * 1024 * 1024), "15.0 MB");
        assert_eq!(format_vault_size(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(
            format_vault_size(2 * 1024 * 1024 * 1024 + 512 * 1024 * 1024),
            "2.5 GB"
        );
    }

    #[test]
    fn test_compute_vault_size_skips_git() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::write(root.join("note1.tmt"), "hello world").unwrap(); // 11 bytes
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub").join("note2.tmt"), "test 12345").unwrap(); // 10 bytes

        // Create .git directory with dummy data that should be ignored
        let git_dir = root.join(".git");
        std::fs::create_dir_all(&git_dir).unwrap();
        std::fs::write(git_dir.join("large_pack"), vec![0u8; 10000]).unwrap();

        let size = compute_vault_size(root);
        assert_eq!(size, 21);
    }
}

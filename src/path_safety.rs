use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Normalize a user-supplied path and ensure it stays within an allowed root.
/// Rejects absolute paths and any that escape via `..`.
pub fn normalize_under_root(root: &Path, candidate: &Path) -> Result<PathBuf> {
    if candidate.is_absolute() {
        anyhow::bail!("absolute paths are not allowed: {}", candidate.display());
    }

    let root_canon = root
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", root.display()))?;
    let joined = root_canon.join(candidate);
    let canon = joined
        .canonicalize()
        .with_context(|| format!("failed to canonicalize {}", joined.display()))?;

    if !canon.starts_with(&root_canon) {
        anyhow::bail!(
            "path escapes root ({}): {}",
            root_canon.display(),
            canon.display()
        );
    }

    Ok(canon)
}

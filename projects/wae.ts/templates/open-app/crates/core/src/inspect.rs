//! Portable domain operation used by the CLI, GUI, and skills.
//!
//! `inspect` walks a directory tree without platform branches. It is a real
//! domain operation so the generated application is runnable before the product
//! replaces it with domain-specific capabilities.

use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use crate::{
    error::{Error, Result},
    platform::platform_name,
};

/// Options for one inspection.
#[derive(Debug, Clone, Default)]
pub struct InspectionOptions {
    /// Root directory. Defaults to the current directory.
    pub root: Option<String>,
    /// Maximum reported entries. Defaults to 100.
    pub limit: Option<usize>,
}

/// One filesystem entry.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionEntry {
    /// Absolute or root-relative display path.
    pub path: String,
    /// Entry size in bytes.
    pub bytes: u64,
    /// `file`, `dir`, or `symlink`.
    pub kind: String,
}

/// Inspection result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectionReport {
    /// Inspected root.
    pub root: String,
    /// Runtime platform identity for diagnostics.
    pub platform: String,
    /// Sum of entry sizes.
    pub total_bytes: u64,
    /// Number of entries encountered before truncation.
    pub entry_count: u64,
    /// Whether reporting stopped at the limit.
    pub truncated: bool,
    /// Largest entries first.
    pub entries: Vec<InspectionEntry>,
    /// Scan timestamp.
    pub inspected_at_unix_ms: u64,
}

const DEFAULT_LIMIT: usize = 100;
const DEFAULT_DEPTH: usize = 8;

/// Inspect a directory tree.
pub fn inspect(options: InspectionOptions) -> Result<InspectionReport> {
    let root = match options.root {
        Some(root) if !root.trim().is_empty() => PathBuf::from(root),
        _ => std::env::current_dir().map_err(|error| Error::io(".", error))?,
    };
    let root_text = root.display().to_string();
    let metadata = fs::symlink_metadata(&root).map_err(|error| Error::io(&root_text, error))?;
    if !metadata.is_dir() {
        return Err(Error::message(format!("inspect root is not a directory: {root_text}")));
    }

    let limit = options.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, 2000);
    let mut entries = Vec::new();
    let mut total_bytes = 0u64;
    let mut entry_count = 0u64;
    let mut truncated = false;
    walk(&root, 0, limit, &mut entries, &mut total_bytes, &mut entry_count, &mut truncated)?;

    entries.sort_by_key(|left| std::cmp::Reverse(left.bytes));
    Ok(InspectionReport {
        root: root_text,
        platform: platform_name().to_string(),
        total_bytes,
        entry_count,
        truncated,
        entries,
        inspected_at_unix_ms: now_ms(),
    })
}

#[allow(clippy::too_many_arguments)]
fn walk(
    dir: &Path,
    depth: usize,
    limit: usize,
    entries: &mut Vec<InspectionEntry>,
    total_bytes: &mut u64,
    entry_count: &mut u64,
    truncated: &mut bool,
) -> Result<()> {
    if depth >= DEFAULT_DEPTH {
        return Ok(());
    }
    let read = fs::read_dir(dir).map_err(|error| Error::io(dir.display().to_string(), error))?;
    for item in read {
        let item = item.map_err(|error| Error::io(dir.display().to_string(), error))?;
        let path = item.path();
        let metadata = fs::symlink_metadata(&path).map_err(|error| Error::io(path.display().to_string(), error))?;
        let kind = if metadata.file_type().is_symlink() {
            "symlink"
        }
        else if metadata.is_dir() {
            "dir"
        }
        else {
            "file"
        };
        *entry_count += 1;
        let bytes = if metadata.is_file() { metadata.len() } else { 0 };
        *total_bytes += bytes;
        if entries.len() < limit {
            entries.push(InspectionEntry { path: path.display().to_string(), bytes, kind: kind.to_string() });
        }
        else {
            *truncated = true;
        }
        if metadata.is_dir() {
            walk(&path, depth + 1, limit, entries, total_bytes, entry_count, truncated)?;
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|duration| duration.as_millis() as u64).unwrap_or(0)
}

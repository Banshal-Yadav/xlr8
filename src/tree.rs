//! Tree browsing + per-file metadata for the file-browser UI.
//! Tree listings walk one directory level; the flat path list reuses the
//! search index (already a full walk, cached per commit); file meta walks
//! commits newest-first until the blob at `path` differs from the first
//! parent — that commit last touched the file.

use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use serde::Serialize;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct TreeEntry {
    pub name: String,
    pub path: String,
    /// "tree" | "blob" | "commit" (submodule gitlink)
    pub kind: &'static str,
}

#[derive(Serialize)]
pub struct TreeResp {
    pub entries: Vec<TreeEntry>,
    pub took_ms: u128,
}

pub fn list_dir(mirror: &PathBuf, refspec: &str, path: &str) -> Result<TreeResp> {
    let t0 = std::time::Instant::now();
    let path = path.trim_matches('/');
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let commit = crate::diff::commit_of(&repo, refspec)?;
    let tree = commit.tree()?;
    let dir = if path.is_empty() {
        tree
    } else {
        let Some(entry) = tree.lookup_entry_by_path(path).map_err(|_| {
            crate::diff::not_found(format!("path not found (or crosses a submodule): {path}"))
        })? else {
            return Err(crate::diff::not_found(format!("path not found: {path}")));
        };
        if !entry.mode().is_tree() {
            return Err(crate::diff::not_found(format!("not a directory: {path}")));
        }
        entry.object()?.try_into_tree()?
    };
    let mut entries = Vec::new();
    for entry in dir.iter() {
        let entry = entry?;
        let mode = entry.mode();
        let kind = if mode.is_tree() {
            "tree"
        } else if mode.is_commit() {
            "commit"
        } else {
            "blob"
        };
        let name_bytes = entry.filename().as_bytes();
        // non-UTF-8 names can't round-trip through query strings — skip
        // rather than emit a path every follow-up request would 404 on
        let Ok(name) = std::str::from_utf8(name_bytes) else {
            continue;
        };
        let name = name.to_string();
        let full = if path.is_empty() {
            name.clone()
        } else {
            format!("{path}/{name}")
        };
        entries.push(TreeEntry {
            name,
            path: full,
            kind,
        });
    }
    // dirs + submodule gitlinks first, then files; case-insensitive alpha — VS Code default
    entries.sort_by(|a, b| {
        let ka = if a.kind == "blob" { 1 } else { 0 };
        let kb = if b.kind == "blob" { 1 } else { 0 };
        ka.cmp(&kb).then_with(|| {
            a.name
                .to_ascii_lowercase()
                .cmp(&b.name.to_ascii_lowercase())
        })
    });
    Ok(TreeResp {
        entries,
        took_ms: t0.elapsed().as_millis(),
    })
}

/// Every blob path at `ref`, sorted — served from the search path index
/// (one tree walk, cached per commit id).
pub fn all_paths(mirror: &PathBuf, refspec: &str) -> Result<Vec<String>> {
    crate::search::all_paths(mirror, refspec)
}

#[derive(Serialize)]
pub struct LastCommit {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

#[derive(Serialize)]
pub struct FileMeta {
    pub path: String,
    pub size: u64,
    pub lines: u64,
    pub binary: bool,
    pub last: Option<LastCommit>,
    pub walked: usize,
    pub took_ms: u128,
}

/// commits scanned before giving up on finding the last toucher
const WALK_BUDGET: usize = 8000;

pub fn file_meta(mirror: &PathBuf, refspec: &str, path: &str) -> Result<FileMeta> {
    let t0 = std::time::Instant::now();
    let path = path.trim_matches('/');
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let root = crate::diff::commit_of(&repo, refspec)?;
    let tree = root.tree()?;
    let Some(entry) = tree
        .lookup_entry_by_path(path)
        .map_err(|_| {
            crate::diff::not_found(format!("path not found (or crosses a submodule): {path}"))
        })?
    else {
        return Err(crate::diff::not_found(format!("path not found: {path}")));
    };
    let mode = entry.mode();
    if mode.is_tree() {
        return Err(crate::diff::not_found(format!("not a file: {path}")));
    }
    let (size, lines, binary) = if mode.is_commit() {
        (0u64, 0u64, false)
    } else {
        let Some(data) = crate::diff::blob_bytes(mirror, &repo, entry.object_id())? else {
            return Err(crate::diff::not_found(format!("not a blob: {path}")));
        };
        crate::diff::blob_facts(&data)
    };

    // git log -1 -- path semantics: walk newest-first; at each commit, if any
    // parent is TREESAME (same blob at path) don't report it and follow only
    // the TREESAME parents; when no parent is TREESAME this commit introduced
    // the current state — that's the last toucher. Date-ordered frontier.
    let mut last: Option<LastCommit> = None;
    let mut walked = 0usize;
    let mut seen = std::collections::HashSet::new();
    let mut heap: std::collections::BinaryHeap<(i64, gix::ObjectId)> =
        std::collections::BinaryHeap::new();
    let root_id = root.id().detach();
    seen.insert(root_id);
    heap.push((root.time().map(|t| t.seconds).unwrap_or(0), root_id));
    let oid_at = |commit: &gix::Commit<'_>| -> Result<Option<gix::ObjectId>> {
        Ok(commit
            .tree()?
            .lookup_entry_by_path(path)?
            .map(|e| e.object_id()))
    };
    while let Some((_, id)) = heap.pop() {
        if walked >= WALK_BUDGET {
            break;
        }
        walked += 1;
        let commit = repo
            .find_object(id)?
            .try_into_commit()
            .context("walk target is not a commit")?;
        let cur = oid_at(&commit)?;
        let mut parents: Vec<gix::ObjectId> = Vec::new();
        let mut treesame = 0usize;
        for pid in commit.parent_ids() {
            let pid = pid.detach();
            let parent = repo.find_object(pid)?.try_into_commit()?;
            if oid_at(&parent)? == cur {
                treesame += 1;
            }
            parents.push(pid);
        }
        if treesame > 0 {
            // follow only the TREESAME parents — a discarded merge side
            // must never be reported as the last toucher
            for pid in parents {
                let parent = repo.find_object(pid)?.try_into_commit()?;
                if oid_at(&parent)? == cur && seen.insert(pid) {
                    heap.push((parent.time().map(|t| t.seconds).unwrap_or(0), pid));
                }
            }
            continue;
        }
        let full = commit.id().to_string();
        let summary = commit
            .message_raw()
            .map(|m| String::from_utf8_lossy(m.as_bytes()))
            .unwrap_or_default();
        let author = commit
            .author()
            .map(|a| String::from_utf8_lossy(a.name.as_bytes()).to_string())
            .unwrap_or_default();
        last = Some(LastCommit {
            short_id: full[..8.min(full.len())].to_string(),
            id: full,
            summary: summary.lines().next().unwrap_or("").to_string(),
            author,
            time: commit.time().map(|t| t.seconds).unwrap_or(0),
        });
        break;
    }

    Ok(FileMeta {
        path: path.to_string(),
        size,
        lines,
        binary,
        last,
        walked,
        took_ms: t0.elapsed().as_millis(),
    })
}

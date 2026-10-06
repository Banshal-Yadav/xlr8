use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::object::tree::diff::ChangeDetached as Change;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Serialize, Clone)]
pub struct FileStat {
    pub path: String,
    pub old_path: Option<String>,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    pub binary: bool,
}

#[derive(Serialize)]
pub struct DiffSummary {
    pub base: String,
    pub head: String,
    pub files: Vec<FileStat>,
    pub total_additions: u32,
    pub total_deletions: u32,
}

#[derive(Serialize)]
pub struct HunkLine {
    pub kind: String,
    pub text: String,
}

#[derive(Serialize)]
pub struct FileDiff {
    pub path: String,
    pub lines: Vec<HunkLine>,
}

fn open_repo(path: &PathBuf) -> Result<gix::Repository> {
    gix::open(path).with_context(|| format!("open repo {}", path.display()))
}

fn commit_of<'r>(repo: &'r gix::Repository, spec: &str) -> Result<gix::Commit<'r>> {
    let id = repo
        .rev_parse_single(spec)
        .with_context(|| format!("resolve {spec}"))?;
    repo.find_object(id.detach())
        .with_context(|| format!("load {spec}"))?
        .peel_to_commit()
        .with_context(|| format!("{spec} is not a commit"))
}

fn blob_bytes(repo: &gix::Repository, id: gix::ObjectId) -> Result<Option<Vec<u8>>> {
    let mut obj = repo.find_object(id)?;
    if !obj.kind.is_blob() {
        return Ok(None);
    }
    Ok(Some(std::mem::take(&mut obj.data)))
}

/// Phase 1: full file list + line stats for a ref range (base..head).
/// Single tree walk, then per-file blob stats fanned out over worker threads.
pub fn compare(mirror: &PathBuf, base: &str, head: &str) -> Result<DiffSummary> {
    let repo = open_repo(mirror)?;
    let base_commit = commit_of(&repo, base)?;
    let head_commit = commit_of(&repo, head)?;
    let base_tree = base_commit.tree()?;
    let head_tree = head_commit.tree()?;

    let t0 = std::time::Instant::now();
    let changes: Vec<(usize, Change)> = repo
        .diff_tree_to_tree(Some(&base_tree), Some(&head_tree), None::<gix::diff::Options>)?
        .into_iter()
        .enumerate()
        .collect();

    let n_threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .clamp(1, 8);
    let chunk = changes.len().div_ceil(n_threads).max(1);
    let t1 = std::time::Instant::now();

    let mut handles = Vec::new();
    let mut rest = changes.into_iter();
    loop {
        let part: Vec<(usize, Change)> = rest.by_ref().take(chunk).collect();
        if part.is_empty() {
            break;
        }
        let mp = mirror.clone();
        handles.push(std::thread::spawn(move || stat_chunk(&mp, part)));
    }

    let mut collected: Vec<(usize, FileStat)> = Vec::new();
    for h in handles {
        let part = h
            .join()
            .map_err(|_| anyhow::anyhow!("stats worker panicked"))??;
        collected.extend(part);
    }
    collected.sort_by_key(|(i, _)| *i);

    let mut files = Vec::with_capacity(collected.len());
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;
    for (_, stat) in collected {
        total_additions += stat.additions;
        total_deletions += stat.deletions;
        files.push(stat);
    }
    tracing::info!(
        "compare total {:?} (incl. stats workers {:?}) for {} files",
        t0.elapsed(),
        t1.elapsed(),
        files.len()
    );

    Ok(DiffSummary {
        base: base.to_string(),
        head: head.to_string(),
        files,
        total_additions,
        total_deletions,
    })
}

fn stat_chunk(mirror: &PathBuf, changes: Vec<(usize, Change)>) -> Result<Vec<(usize, FileStat)>> {
    let repo = open_repo(mirror)?;
    let mut out = Vec::with_capacity(changes.len());
    for (idx, change) in changes {
        let (path, old_path, status, old_id, new_id, is_tree) = match change {
            Change::Addition {
                location,
                entry_mode,
                id,
                ..
            } => (
                String::from_utf8_lossy(&location).to_string(),
                None,
                "A",
                None,
                Some(id),
                entry_mode.is_tree(),
            ),
            Change::Deletion {
                location,
                entry_mode,
                id,
                ..
            } => (
                String::from_utf8_lossy(&location).to_string(),
                None,
                "D",
                Some(id),
                None,
                entry_mode.is_tree(),
            ),
            Change::Modification {
                location,
                entry_mode,
                previous_id,
                id,
                ..
            } => (
                String::from_utf8_lossy(&location).to_string(),
                None,
                "M",
                Some(previous_id),
                Some(id),
                entry_mode.is_tree(),
            ),
            Change::Rewrite {
                location,
                entry_mode,
                source_location,
                source_id,
                id,
                copy,
                ..
            } => (
                String::from_utf8_lossy(&location).to_string(),
                Some(String::from_utf8_lossy(&source_location).to_string()),
                if copy { "C" } else { "R" },
                Some(source_id),
                Some(id),
                entry_mode.is_tree(),
            ),
        };

        if is_tree {
            continue;
        }

        let (additions, deletions, binary) = match (old_id, new_id) {
            (None, Some(new)) => (count_lines(&repo, new)?, 0, false),
            (Some(old), None) => (0, count_lines(&repo, old)?, false),
            (Some(old), Some(new)) => {
                let old_data = blob_bytes(&repo, old)?;
                let new_data = blob_bytes(&repo, new)?;
                if old_data.as_deref().is_some_and(|d| d.contains(&0))
                    || new_data.as_deref().is_some_and(|d| d.contains(&0))
                {
                    (0, 0, true)
                } else {
                    let old_text = String::from_utf8_lossy(old_data.as_deref().unwrap_or(b""));
                    let new_text = String::from_utf8_lossy(new_data.as_deref().unwrap_or(b""));
                    let (a, d) = line_stats(&old_text, &new_text);
                    (a, d, false)
                }
            }
            (None, None) => (0, 0, false),
        };
        out.push((
            idx,
            FileStat {
                path,
                old_path,
                status: status.to_string(),
                additions,
                deletions,
                binary,
            },
        ));
    }
    Ok(out)
}

/// Phase 2: per-file line diff, computed on demand.
pub fn file_diff(mirror: &PathBuf, base: &str, head: &str, path: &str) -> Result<FileDiff> {
    let repo = open_repo(mirror)?;
    let base_commit = commit_of(&repo, base)?;
    let head_commit = commit_of(&repo, head)?;
    let base_tree = base_commit.tree()?;
    let head_tree = head_commit.tree()?;

    let old_id = base_tree
        .lookup_entry_by_path(path)?
        .map(|e| e.object_id());
    let new_id = head_tree
        .lookup_entry_by_path(path)?
        .map(|e| e.object_id());
    if old_id.is_none() && new_id.is_none() {
        anyhow::bail!("path not found in either ref: {path}");
    }

    let old_bytes: Option<Vec<u8>> = match old_id {
        Some(id) => blob_bytes(&repo, id)?,
        None => None,
    };
    let new_bytes: Option<Vec<u8>> = match new_id {
        Some(id) => blob_bytes(&repo, id)?,
        None => None,
    };
    let old_text = String::from_utf8_lossy(old_bytes.as_deref().unwrap_or(b"")).to_string();
    let new_text = String::from_utf8_lossy(new_bytes.as_deref().unwrap_or(b"")).to_string();

    let mut lines = Vec::new();
    for (kind, text) in diff_lines(&old_text, &new_text) {
        lines.push(HunkLine { kind, text });
    }
    Ok(FileDiff {
        path: path.to_string(),
        lines,
    })
}

fn count_lines(repo: &gix::Repository, id: gix::ObjectId) -> Result<u32> {
    let data = blob_bytes(repo, id)?.unwrap_or_default();
    Ok(String::from_utf8_lossy(&data).lines().count() as u32)
}

/// Count-only line diff (phase 1): same algorithm as diff_lines, zero per-line allocation.
fn line_stats(old: &str, new: &str) -> (u32, u32) {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut additions = 0u32;
    let mut deletions = 0u32;
    let mut i = 0usize;
    let mut j = 0usize;
    while i < old_lines.len() && j < new_lines.len() {
        if old_lines[i] == new_lines[j] {
            i += 1;
            j += 1;
            continue;
        }
        let window = 80usize;
        let mut matched: Option<(usize, usize)> = None;
        let old_end = (i + 1 + window).min(old_lines.len());
        let new_end = (j + 1 + window).min(new_lines.len());
        'scan: for oi in (i + 1)..old_end {
            for ni in (j + 1)..new_end {
                if old_lines[oi] == new_lines[ni] {
                    matched = Some((oi, ni));
                    break 'scan;
                }
            }
        }
        match matched {
            Some((oi, ni)) => {
                deletions += (oi - i) as u32;
                additions += (ni - j) as u32;
                i = oi;
                j = ni;
            }
            None => {
                additions += 1;
                deletions += 1;
                i += 1;
                j += 1;
            }
        }
    }
    deletions += (old_lines.len() - i) as u32;
    additions += (new_lines.len() - j) as u32;
    (additions, deletions)
}

/// Line diff: equality by line content. Returns (kind, text) where kind in add/del/ctx.
pub fn diff_lines(old: &str, new: &str) -> Vec<(String, String)> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let mut out = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < old_lines.len() && j < new_lines.len() {
        if old_lines[i] == new_lines[j] {
            out.push(("ctx".into(), old_lines[i].to_string()));
            i += 1;
            j += 1;
            continue;
        }
        let window = 80usize;
        let mut matched: Option<(usize, usize)> = None;
        let old_end = (i + 1 + window).min(old_lines.len());
        let new_end = (j + 1 + window).min(new_lines.len());
        'scan: for oi in (i + 1)..old_end {
            for ni in (j + 1)..new_end {
                if old_lines[oi] == new_lines[ni] {
                    matched = Some((oi, ni));
                    break 'scan;
                }
            }
        }
        match matched {
            Some((oi, ni)) => {
                for k in i..oi {
                    out.push(("del".into(), old_lines[k].to_string()));
                }
                for k in j..ni {
                    out.push(("add".into(), new_lines[k].to_string()));
                }
                i = oi;
                j = ni;
            }
            None => {
                out.push(("del".into(), old_lines[i].to_string()));
                out.push(("add".into(), new_lines[j].to_string()));
                i += 1;
                j += 1;
            }
        }
    }
    for k in i..old_lines.len() {
        out.push(("del".into(), old_lines[k].to_string()));
    }
    for k in j..new_lines.len() {
        out.push(("add".into(), new_lines[k].to_string()));
    }
    out
}

/// Commit log entries for the web UI.
#[derive(Serialize)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
}

pub fn commits(mirror: &PathBuf, limit: usize, refspec: Option<&str>) -> Result<Vec<CommitInfo>> {
    let repo = open_repo(mirror)?;
    let spec = refspec.unwrap_or("HEAD");
    let mut out = Vec::new();
    let mut next = Some(commit_of(&repo, spec)?);
    while let Some(commit) = next {
        if out.len() >= limit {
            break;
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
        let time = commit.time().map(|t| t.seconds).unwrap_or(0);
        out.push(CommitInfo {
            short_id: full[..8.min(full.len())].to_string(),
            id: full,
            summary: summary.lines().next().unwrap_or("").to_string(),
            author,
            time,
        });
        next = match commit.parent_ids().next() {
            Some(parent) => Some(
                repo.find_object(parent.detach())?
                    .try_into_commit()
                    .context("parent is not a commit")?,
            ),
            None => None,
        };
    }
    Ok(out)
}

/// Branch/tag refs for selectors.
#[derive(Serialize)]
pub struct RefInfo {
    pub name: String,
    pub id: String,
}

pub fn refs(mirror: &PathBuf) -> Result<Vec<RefInfo>> {
    let repo = open_repo(mirror)?;
    let mut seen: HashMap<String, String> = HashMap::new();
    for r in repo.references()?.all()? {
        let Ok(r) = r else {
            continue;
        };
        let full = String::from_utf8_lossy(r.name().as_bstr().as_bytes()).to_string();
        let short = full
            .trim_start_matches("refs/heads/")
            .trim_start_matches("refs/tags/")
            .to_string();
        if short != full || full.starts_with("refs/heads/") || full.starts_with("refs/tags/") {
            if let Ok(id) = r.into_fully_peeled_id() {
                seen.insert(short, id.to_string());
            }
        }
    }
    let mut v: Vec<RefInfo> = seen
        .into_iter()
        .map(|(name, id)| RefInfo { name, id })
        .collect();
    v.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(v)
}

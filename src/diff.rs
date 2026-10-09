use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use gix::object::tree::diff::ChangeDetached as Change;
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{OnceLock, RwLock};

#[derive(Debug)]
pub struct NotFound(pub String);

impl std::fmt::Display for NotFound {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for NotFound {}

pub(crate) fn not_found(msg: impl Into<String>) -> anyhow::Error {
    NotFound(msg.into()).into()
}

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
    pub kind: &'static str,
    pub text: String,
}

#[derive(Serialize)]
pub struct Hunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    pub lines: Vec<HunkLine>,
}

#[derive(Serialize)]
pub struct FileDiff {
    pub path: String,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
}

pub(crate) fn commit_of<'r>(repo: &'r gix::Repository, spec: &str) -> Result<gix::Commit<'r>> {
    let id = repo
        .rev_parse_single(spec)
        .map_err(|_| not_found(format!("unknown ref: {spec}")))?;
    repo.find_object(id.detach())
        .with_context(|| format!("load {spec}"))?
        .peel_to_commit()
        .with_context(|| format!("{spec} is not a commit"))
}

fn blob_bytes(
    mirror: &std::path::Path,
    repo: &gix::Repository,
    id: gix::ObjectId,
) -> Result<Option<std::sync::Arc<[u8]>>> {
    if let Some(b) = crate::repo::blob_get(mirror, id) {
        return Ok(Some(b));
    }
    let mut obj = repo.find_object(id)?;
    if !obj.kind.is_blob() {
        return Ok(None);
    }
    let data: std::sync::Arc<[u8]> = std::mem::take(&mut obj.data).into();
    crate::repo::blob_put(mirror, id, data.clone());
    Ok(Some(data))
}

// git's buffer_is_binary(): NUL within the first 8000 bytes only.
pub(crate) fn has_nul(d: &[u8]) -> bool {
    d.iter().take(8000).any(|&b| b == 0)
}

pub fn compare(mirror: &PathBuf, base: &str, head: &str) -> Result<DiffSummary> {
    let t0 = std::time::Instant::now();
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let base_commit = commit_of(&repo, base)?;
    let head_commit = commit_of(&repo, head)?;
    let base_tree = base_commit.tree()?;
    let head_tree = head_commit.tree()?;

    let t_walk = std::time::Instant::now();
    let mut changes: Vec<(usize, Change)> = repo
        .diff_tree_to_tree(
            Some(&base_tree),
            Some(&head_tree),
            gix::diff::Options::default(),
        )?
        .into_iter()
        .enumerate()
        .collect();
    let t_walk = t_walk.elapsed();

    let t_rename = std::time::Instant::now();
    crate::rewrite::detect(&tsr, mirror, &mut changes)?;
    let t_rename = t_rename.elapsed();

    let t_stats = std::time::Instant::now();
    let collected = fill_stats(mirror, &tsr, changes)?;

    let mut files = Vec::with_capacity(collected.len());
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;
    for (_, stat) in collected {
        total_additions += stat.additions;
        total_deletions += stat.deletions;
        files.push(stat);
    }
    crate::debug!(
        "compare total {:?} (walk {:?}, renames {:?}, stats {:?}) for {} files",
        t0.elapsed(),
        t_walk,
        t_rename,
        t_stats.elapsed(),
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

/// Fine-grained work queue shared by compare() and per-commit stats: uneven
/// core speeds don't stall the wall clock; tiny diffs run inline (thread spawn
/// would cost more than the stats themselves). Returns stats keyed by the
/// caller's tuple index — commits pass their commit index, compare passes
/// the change position.
fn fill_stats(
    mirror: &std::path::PathBuf,
    tsr: &std::sync::Arc<gix::ThreadSafeRepository>,
    changes: Vec<(usize, Change)>,
) -> Result<Vec<(usize, FileStat)>> {
    const STAT_CHUNK: usize = 16;
    let mut collected: Vec<(usize, FileStat)> = Vec::new();
    if changes.len() <= STAT_CHUNK {
        if !changes.is_empty() {
            collected = stat_chunk(mirror.clone(), tsr.clone(), changes)?;
        }
    } else {
        let n_chunks = changes.len().div_ceil(STAT_CHUNK);
        let n_threads = crate::repo::n_threads().min(n_chunks);
        let data = std::sync::Arc::new(changes);
        let next = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut handles = Vec::with_capacity(n_threads);
        for _ in 0..n_threads {
            let (tsr, mirror, data, next) =
                (tsr.clone(), mirror.clone(), data.clone(), next.clone());
            handles.push(std::thread::spawn(move || {
                let mut out = Vec::new();
                loop {
                    let s = next.fetch_add(STAT_CHUNK, std::sync::atomic::Ordering::Relaxed);
                    if s >= data.len() {
                        break;
                    }
                    let e = (s + STAT_CHUNK).min(data.len());
                    out.extend(stat_chunk(mirror.clone(), tsr.clone(), data[s..e].to_vec())?);
                }
                anyhow::Ok(out)
            }));
        }

        for h in handles {
            let part = h
                .join()
                .map_err(|_| anyhow::anyhow!("stats worker panicked"))??;
            collected.extend(part);
        }
    }
    collected.sort_by_key(|(i, _)| *i);
    Ok(collected)
}

fn stat_chunk(
    mirror: std::path::PathBuf,
    tsr: std::sync::Arc<gix::ThreadSafeRepository>,
    changes: Vec<(usize, Change)>,
) -> Result<Vec<(usize, FileStat)>> {
    let repo = crate::repo::handle(&tsr);
    let mut out = Vec::with_capacity(changes.len());
    for (idx, change) in changes {
        let (path, old_path, status, old_id, new_id, is_tree, is_gitlink) = match change {
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
                entry_mode.is_commit(),
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
                entry_mode.is_commit(),
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
                entry_mode.is_commit(),
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
                entry_mode.is_commit(),
            ),
        };

        if is_tree {
            continue;
        }

        let same_blob = matches!((&old_id, &new_id), (Some(a), Some(b)) if a == b);
        let (additions, deletions, binary) = if same_blob {
            (0, 0, false)
        } else if is_gitlink {
            // submodule gitlink: the commit lives in another object database —
            // no content here. git counts its single "Subproject commit" line:
            // +1 on add, −1 on delete, 1/1 on change.
            match (&old_id, &new_id) {
                (Some(a), Some(b)) if a != b => (1, 1, false),
                (None, Some(_)) => (1, 0, false),
                (Some(_), None) => (0, 1, false),
                _ => (0, 0, false),
            }
        } else {
            let old_bytes = match old_id {
                Some(id) => blob_bytes(&mirror, &repo, id)?,
                None => None,
            };
            let new_bytes = match new_id {
                Some(id) => blob_bytes(&mirror, &repo, id)?,
                None => None,
            };
            let binary = old_bytes.as_deref().is_some_and(has_nul)
                || new_bytes.as_deref().is_some_and(has_nul);
            if binary {
                (0, 0, true)
            } else {
                let (additions, deletions) = match (&old_bytes, &new_bytes) {
                    (Some(old), Some(new)) => line_stats(old, new),
                    (None, Some(new)) => {
                        (imara_diff::sources::byte_lines_with_terminator(new).count() as u32, 0)
                    }
                    (Some(old), None) => {
                        (0, imara_diff::sources::byte_lines_with_terminator(old).count() as u32)
                    }
                    (None, None) => (0, 0),
                };
                (additions, deletions, false)
            }
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

const CTX: usize = 3;

pub fn file_diff(
    mirror: &PathBuf,
    base: &str,
    head: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<FileDiff> {
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let base_commit = commit_of(&repo, base)?;
    let head_commit = commit_of(&repo, head)?;
    let base_tree = base_commit.tree()?;
    let head_tree = head_commit.tree()?;

    // renamed files sit under their pre-rename name on the base side
    let old_entry = base_tree.lookup_entry_by_path(old_path.unwrap_or(path))?;
    let new_entry = head_tree.lookup_entry_by_path(path)?;
    let (old_id, old_is_gitlink) = match old_entry {
        Some(e) => (Some(e.object_id()), e.mode().is_commit()),
        None => (None, false),
    };
    let (new_id, new_is_gitlink) = match new_entry {
        Some(e) => (Some(e.object_id()), e.mode().is_commit()),
        None => (None, false),
    };
    if old_id.is_none() && new_id.is_none() {
        return Err(not_found(format!("path not found in either ref: {path}")));
    }

    // submodule gitlink: commit lives in another object database — synthesize
    // git's one-line "Subproject commit" pseudo-diff instead of loading content
    if old_is_gitlink || new_is_gitlink {
        let sub = |id: &Option<gix::ObjectId>| {
            id.map(|i| format!("Subproject commit {i}")).unwrap_or_default()
        };
        let (lines, old_start, old_count, new_start, new_count) = match (&old_id, &new_id) {
            (Some(a), Some(b)) if a != b => (
                vec![
                    HunkLine {
                        kind: "del",
                        text: sub(&old_id),
                    },
                    HunkLine {
                        kind: "add",
                        text: sub(&new_id),
                    },
                ],
                1,
                1,
                1,
                1,
            ),
            (None, Some(_)) => (
                vec![HunkLine {
                    kind: "add",
                    text: sub(&new_id),
                }],
                0,
                0,
                1,
                1,
            ),
            (Some(_), None) => (
                vec![HunkLine {
                    kind: "del",
                    text: sub(&old_id),
                }],
                1,
                1,
                0,
                0,
            ),
            _ => (Vec::new(), 0, 0, 0, 0),
        };
        let hunks = if lines.is_empty() {
            Vec::new()
        } else {
            vec![Hunk {
                old_start,
                old_count,
                new_start,
                new_count,
                lines,
            }]
        };
        return Ok(FileDiff {
            path: path.to_string(),
            binary: false,
            hunks,
        });
    }

    let old_bytes: Option<std::sync::Arc<[u8]>> = match old_id {
        Some(id) => blob_bytes(mirror, &repo, id)?,
        None => None,
    };
    let new_bytes: Option<std::sync::Arc<[u8]>> = match new_id {
        Some(id) => blob_bytes(mirror, &repo, id)?,
        None => None,
    };
    let binary =
        old_bytes.as_deref().is_some_and(has_nul) || new_bytes.as_deref().is_some_and(has_nul);
    if binary {
        return Ok(FileDiff {
            path: path.to_string(),
            binary: true,
            hunks: Vec::new(),
        });
    }

    let old_text = String::from_utf8_lossy(old_bytes.as_deref().unwrap_or(b""));
    let new_text = String::from_utf8_lossy(new_bytes.as_deref().unwrap_or(b""));
    let hunks = build_hunks(diff_lines(&old_text, &new_text));
    Ok(FileDiff {
        path: path.to_string(),
        binary: false,
        hunks,
    })
}

/// Group diff rows into hunks with 3 lines of context, GitHub-style.
fn build_hunks(rows: Vec<(&'static str, String)>) -> Vec<Hunk> {
    // per row: (displayed old no, displayed new no, old lines consumed, new lines consumed)
    let mut meta: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(rows.len());
    let (mut o, mut n) = (1u32, 1u32);
    for (kind, _) in &rows {
        let (d_old, d_new) = match *kind {
            "add" => {
                let d = (0, n);
                n += 1;
                d
            }
            "del" => {
                let d = (o, 0);
                o += 1;
                d
            }
            _ => {
                let d = (o, n);
                o += 1;
                n += 1;
                d
            }
        };
        meta.push((d_old, d_new, o, n));
    }

    let mut groups: Vec<(usize, usize)> = Vec::new();
    for (i, (kind, _)) in rows.iter().enumerate() {
        if *kind == "ctx" {
            continue;
        }
        match groups.last_mut() {
            Some(g) if i - g.1 <= 2 * CTX + 1 => g.1 = i,
            _ => groups.push((i, i)),
        }
    }
    if groups.is_empty() {
        return Vec::new();
    }

    let mut hunks = Vec::with_capacity(groups.len());
    let mut rest = rows;
    let mut consumed = 0usize;
    for (first, last) in groups {
        let start = first.saturating_sub(CTX).max(consumed);
        let end = (last + 1 + CTX).min(consumed + rest.len());
        let seg: Vec<_> = rest.drain(0..end - consumed).collect();
        let skip = start - consumed;
        consumed = end;

        let range = &meta[start..end];
        let old_count = range.iter().filter(|m| m.0 != 0).count();
        let new_count = range.iter().filter(|m| m.1 != 0).count();
        let old_start = if old_count > 0 {
            range
                .iter()
                .find_map(|m| (m.0 != 0).then_some(m.0 as usize))
                .unwrap()
        } else if start > 0 {
            meta[start - 1].2 as usize
        } else {
            0
        };
        let new_start = if new_count > 0 {
            range
                .iter()
                .find_map(|m| (m.1 != 0).then_some(m.1 as usize))
                .unwrap()
        } else if start > 0 {
            meta[start - 1].3 as usize
        } else {
            0
        };

        let lines = seg
            .into_iter()
            .skip(skip)
            .map(|(kind, text)| HunkLine { kind, text })
            .collect();
        hunks.push(Hunk {
            old_start,
            old_count,
            new_start,
            new_count,
            lines,
        });
    }
    hunks
}

/// Byte-line tokens with terminators kept: an EOF-newline-only change counts
/// +1/−1 like git.
fn line_stats(old: &[u8], new: &[u8]) -> (u32, u32) {
    let input = imara_diff::intern::InternedInput::new(
        imara_diff::sources::byte_lines_with_terminator(old),
        imara_diff::sources::byte_lines_with_terminator(new),
    );
    let mut additions = 0u32;
    let mut deletions = 0u32;
    imara_diff::diff(
        imara_diff::Algorithm::Myers,
        &input,
        |before: std::ops::Range<u32>, after: std::ops::Range<u32>| {
            deletions += (before.end - before.start) as u32;
            additions += (after.end - after.start) as u32;
        },
    );
    (additions, deletions)
}

/// Line tokens with terminators kept (EOF-newline changes are real changes);
/// rows are (kind, text) with the trailing newline stripped for display.
pub fn diff_lines(old: &str, new: &str) -> Vec<(&'static str, String)> {
    let input = imara_diff::intern::InternedInput::new(
        imara_diff::sources::lines_with_terminator(old),
        imara_diff::sources::lines_with_terminator(new),
    );
    fn text(s: &str) -> String {
        let t = s
            .strip_suffix("\r\n")
            .or_else(|| s.strip_suffix('\n'))
            .unwrap_or(s);
        t.to_string()
    }
    let mut changes: Vec<(std::ops::Range<u32>, std::ops::Range<u32>)> = Vec::new();
    imara_diff::diff(imara_diff::Algorithm::Myers, &input, |before, after| {
        changes.push((before, after));
    });
    let mut out: Vec<(&'static str, String)> = Vec::new();
    let mut cursor = 0u32;
    for (before, after) in changes {
        while cursor < before.start {
            out.push((
                "ctx",
                text(input.interner[input.before[cursor as usize]]),
            ));
            cursor += 1;
        }
        for i in before.clone() {
            out.push(("del", text(input.interner[input.before[i as usize]])));
        }
        for i in after {
            out.push(("add", text(input.interner[input.after[i as usize]])));
        }
        cursor = before.end;
    }
    while cursor < input.before.len() as u32 {
        out.push((
            "ctx",
            text(input.interner[input.before[cursor as usize]]),
        ));
        cursor += 1;
    }
    out
}

#[derive(Serialize)]
pub struct CommitInfo {
    pub id: String,
    pub short_id: String,
    pub summary: String,
    pub author: String,
    pub time: i64,
    /// 0 unless requested with `stats=1` (second, cached pass — the list
    /// itself never waits on diff computation)
    #[serde(default)]
    pub additions: u32,
    #[serde(default)]
    pub deletions: u32,
}

pub fn commits(
    mirror: &PathBuf,
    limit: usize,
    refspec: Option<&str>,
    want_stats: bool,
    offset: usize,
) -> Result<Vec<CommitInfo>> {
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let spec = refspec.unwrap_or("HEAD");
    let root = commit_of(&repo, spec)?;

    // git-log-style walk: every reachable parent, newest committer date first
    let mut seen = std::collections::HashSet::new();
    let mut heap: std::collections::BinaryHeap<(i64, gix::ObjectId)> =
        std::collections::BinaryHeap::new();
    let root_id = root.id().detach();
    seen.insert(root_id);
    heap.push((root.time().map(|t| t.seconds).unwrap_or(0), root_id));

    let mut out = Vec::new();
    let mut ids: Vec<gix::ObjectId> = Vec::new();
    let mut parents: Vec<Option<gix::ObjectId>> = Vec::new();
    // pagination: walk `offset` commits past without collecting them (their
    // parents still feed the heap — that's the walk), then collect `limit`
    let mut skipped = 0usize;
    while let Some((_, id)) = heap.pop() {
        if skipped >= offset && out.len() >= limit {
            break;
        }
        let commit = repo
            .find_object(id)?
            .try_into_commit()
            .context("walk target is not a commit")?;
        if skipped < offset {
            skipped += 1;
        } else {
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
            let p0 = commit.parent_ids().next().map(|p| p.detach());
            ids.push(id);
            parents.push(p0);
            out.push(CommitInfo {
                short_id: full[..8.min(full.len())].to_string(),
                id: full,
                summary: summary.lines().next().unwrap_or("").to_string(),
                author,
                time,
                additions: 0,
                deletions: 0,
            });
        }
        for pid in commit.parent_ids() {
            let pid = pid.detach();
            if seen.insert(pid) {
                let parent = repo
                    .find_object(pid)?
                    .try_into_commit()
                    .context("parent is not a commit")?;
                heap.push((parent.time().map(|t| t.seconds).unwrap_or(0), pid));
            }
        }
    }

    // stats=1 second pass: per-commit +/−, all commits flattened into one
    // parallel fill_stats batch; merge commits diff vs their first parent
    // (GitHub-style). Not computed in the plain list call — that path stays
    // a ~20 ms walk. Tree-diff + rename detection runs in parallel per commit.
    if want_stats && !out.is_empty() {
        let n_ids = ids.len();
        let n_threads = crate::repo::n_threads().min(n_ids).max(1);
        let chunk = n_ids.div_ceil(n_threads).max(1);
        let ids_arc = std::sync::Arc::new(ids);
        let parents_arc = std::sync::Arc::new(parents);
        let mut all: Vec<(usize, Change)> = Vec::new();
        let mut handles = Vec::new();
        for start in (0..n_ids).step_by(chunk) {
            let end = (start + chunk).min(n_ids);
            let tsr = tsr.clone();
            let mirror = mirror.clone();
            let ids_c = ids_arc.clone();
            let parents_c = parents_arc.clone();
            handles.push(std::thread::spawn(move || {
                let repo = crate::repo::handle(&tsr);
                let mut part: Vec<(usize, Change)> = Vec::new();
                for i in start..end {
                    let head = repo.find_object(ids_c[i])?.try_into_commit()?;
                    let head_tree = head.tree()?;
                    let parent_tree = match parents_c[i] {
                        Some(pid) => Some(repo.find_object(pid)?.try_into_commit()?.tree()?),
                        None => None,
                    };
                    let mut ch: Vec<(usize, Change)> = repo
                        .diff_tree_to_tree(
                            parent_tree.as_ref(),
                            Some(&head_tree),
                            gix::diff::Options::default(),
                        )?
                        .into_iter()
                        .enumerate()
                        .collect();
                    crate::rewrite::detect(&tsr, &mirror, &mut ch)?;
                    part.extend(ch.into_iter().map(|(_, c)| (i, c)));
                }
                anyhow::Ok(part)
            }));
        }
        for h in handles {
            let part = h
                .join()
                .map_err(|_| anyhow::anyhow!("commit-stats worker panicked"))??;
            all.extend(part);
        }
        let collected = fill_stats(mirror, &tsr, all)?;
        let mut acc: Vec<(u32, u32)> = vec![(0, 0); out.len()];
        for (i, stat) in collected {
            acc[i].0 += stat.additions;
            acc[i].1 += stat.deletions;
        }
        for (i, info) in out.iter_mut().enumerate() {
            info.additions = acc[i].0;
            info.deletions = acc[i].1;
        }
    }
    Ok(out)
}

/// `rev-list --count`: every commit reachable from ref — same graph the
/// paginated walk enumerates, so page total = ceil(count / limit).
///
/// On-disk cache: file format `XLR8CNT1 <key_hash> <count> <checksum>` where
/// key_hash = hash(all refs sorted + HEAD tip + binary version) and
/// checksum  = hash(magic|version|key_hash|count). Atomic write via
/// temp+rename. Any mismatch (magic, version, key, checksum, parse) →
/// ignore file and recompute. Keying on ALL refs means a force-push, sync,
/// or deleted tag changes the key → automatic invalidation.
const CNT_MAGIC: &str = "XLR8CNT1";

fn hash_hex(s: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

/// fingerprint of the mirror state this count is valid for
fn count_key(repo: &gix::Repository, spec: &str) -> String {
    let mut parts: Vec<String> = Vec::new();
    if let Ok(refs) = repo.references() {
        if let Ok(all) = refs.all() {
            for r in all.flatten() {
                let name = String::from_utf8_lossy(r.name().as_bstr().as_bytes()).into_owned();
                let id = r.id().detach().to_string();
                parts.push(format!("{name}={id}"));
            }
        }
    }
    parts.sort();
    if let Ok(head) = repo.head_id() {
        parts.push(format!("HEAD={}", head.detach()));
    }
    parts.push(format!("BIN={}", env!("CARGO_PKG_VERSION")));
    parts.push(format!("SPEC={spec}"));
    hash_hex(&parts.join("\n"))
}

pub fn commit_count(mirror: &PathBuf, refspec: Option<&str>) -> Result<usize> {
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let spec = refspec.unwrap_or("HEAD");
    let root = commit_of(&repo, spec)?;
    let tip = root.id().detach();

    let cache_root = crate::mirror::cache_dir().join("mirrors").join("count-cache");
    let slug = mirror
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cache_dir = cache_root.join(slug);
    let key_hash = count_key(&repo, spec);
    let cache_file = cache_dir.join(&key_hash);

    // valid hit only if magic+version+key+checksum all match
    if let Ok(s) = std::fs::read_to_string(&cache_file) {
        let tok: Vec<&str> = s.split_whitespace().collect();
        if tok.len() == 4 && tok[0] == CNT_MAGIC && tok[1] == key_hash {
            if let Ok(n) = tok[2].parse::<usize>() {
                let recomputed = hash_hex(&format!("{CNT_MAGIC}|{key_hash}|{n}|1"));
                if tok[3] == recomputed {
                    return Ok(n);
                }
            }
        }
        // any mismatch → fall through and recompute (also cleans bad file)
        let _ = std::fs::remove_file(&cache_file);
    }

    let mut seen = std::collections::HashSet::new();
    let mut heap: std::collections::BinaryHeap<(i64, gix::ObjectId)> =
        std::collections::BinaryHeap::new();
    let root_id = tip;
    seen.insert(root_id);
    heap.push((root.time().map(|t| t.seconds).unwrap_or(0), root_id));
    let mut n = 0usize;
    while let Some((_, id)) = heap.pop() {
        n += 1;
        let commit = repo
            .find_object(id)?
            .try_into_commit()
            .context("walk target is not a commit")?;
        for pid in commit.parent_ids() {
            let pid = pid.detach();
            if seen.insert(pid) {
                let parent = repo
                    .find_object(pid)?
                    .try_into_commit()
                    .context("parent is not a commit")?;
                heap.push((parent.time().map(|t| t.seconds).unwrap_or(0), pid));
            }
        }
    }

    // atomic persist: temp + rename; checksum covers magic|key|count|format-ver
    let _ = std::fs::create_dir_all(&cache_dir);
    let ver = "1";
    let checksum = hash_hex(&format!("{CNT_MAGIC}|{key_hash}|{n}|{ver}"));
    let payload = format!("{CNT_MAGIC} {key_hash} {n} {checksum}\n");
    let tmp = cache_dir.join(format!(".{}.tmp.{}", key_hash, std::process::id()));
    if std::fs::write(&tmp, payload).is_ok() {
        let _ = std::fs::rename(&tmp, &cache_file);
    }
    Ok(n)
}

#[derive(Serialize, Clone)]
pub struct RefInfo {
    pub name: String,
    pub id: String,
}

static REF_CACHE: OnceLock<RwLock<HashMap<PathBuf, Vec<RefInfo>>>> = OnceLock::new();

pub(crate) fn clear_ref_cache(path: &std::path::Path) {
    if let Some(c) = REF_CACHE.get() {
        if let Ok(mut m) = c.write() {
            m.remove(path);
        }
    }
}

pub fn refs(mirror: &PathBuf) -> Result<Vec<RefInfo>> {
    let cache = REF_CACHE.get_or_init(Default::default);
    if let Ok(m) = cache.read() {
        if let Some(v) = m.get(mirror) {
            return Ok(v.clone());
        }
    }
    let tsr = crate::repo::get(mirror)?;
    // collect short-name → unpeeled id first (cheap), peel in parallel.
    // HashMap last-wins matches the original dedup when a branch+tag share a short name.
    let mut raw: Vec<(String, gix::ObjectId)> = Vec::new();
    {
        let repo = crate::repo::handle(&tsr);
        for r in repo.references()?.all()? {
            let Ok(r) = r else {
                continue;
            };
            let full = String::from_utf8_lossy(r.name().as_bstr().as_bytes()).to_string();
            let short = full
                .trim_start_matches("refs/heads/")
                .trim_start_matches("refs/tags/")
                .to_string();
            if short != full {
                raw.push((short, r.id().detach()));
            }
        }
    }
    // dedup by short name, last occurrence wins (same as HashMap insert order)
    {
        let mut order: Vec<String> = Vec::with_capacity(raw.len());
        let mut last: HashMap<String, gix::ObjectId> = HashMap::with_capacity(raw.len());
        for (short, id) in raw {
            if !last.contains_key(&short) {
                order.push(short.clone());
            }
            last.insert(short, id);
        }
        raw = order.into_iter().filter_map(|s| last.get(&s).map(|id| (s, *id))).collect();
    }
    let n_threads = crate::repo::n_threads().min(raw.len()).max(1);
    let chunk = raw.len().div_ceil(n_threads).max(1);
    let mut handles = Vec::new();
    for c in raw.chunks(chunk) {
        let c: Vec<(String, gix::ObjectId)> = c.to_vec();
        let tsr = tsr.clone();
        handles.push(std::thread::spawn(move || {
            let repo = crate::repo::handle(&tsr);
            let mut out = Vec::with_capacity(c.len());
            for (short, id) in c {
                // peel tags → first non-tag object of any kind (blob tags like
                // GPG keys are valid refs; into_fully_peeled_id did the same)
                if let Ok(obj) = repo.find_object(id) {
                    let mut cur = obj;
                    while cur.kind == gix::object::Kind::Tag {
                        match cur.to_tag_ref_iter().target_id() {
                            Ok(tid) => match repo.find_object(tid) {
                                Ok(next) => cur = next,
                                Err(_) => break,
                            },
                            Err(_) => break,
                        }
                    }
                    out.push(RefInfo {
                        name: short,
                        id: cur.id().to_string(),
                    });
                }
            }
            out
        }));
    }
    let mut v: Vec<RefInfo> = Vec::with_capacity(raw.len());
    for h in handles {
        if let Ok(part) = h.join() {
            v.extend(part);
        }
    }
    v.sort_by(|a, b| a.name.cmp(&b.name));
    if let Ok(mut m) = cache.write() {
        m.insert(mirror.clone(), v.clone());
    }
    Ok(v)
}

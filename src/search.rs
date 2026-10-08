//! Repo search: a per-ref flat path index (one entry per blob, u64 char
//! mask each) answers fuzzy path queries with one AND plus a subsequence
//! scan; content terms run a budget-bounded grep over the candidates and
//! never return stale bytes (blobs are read fresh). The index is immutable
//! per commit id, so it caches exactly like the mirror.

use anyhow::{Context, Result};
use gix::bstr::ByteSlice;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// grep skips blobs bigger than this — keeps the budget honest on repos
/// that vendor big generated files
const MAX_FILE: u64 = 1 << 20;
const GREP_BUDGET: Duration = Duration::from_millis(1200);
const MAX_FILES_OUT: usize = 200;
const MAX_HITS_OUT: usize = 200;
const MAX_HITS_PER_FILE: usize = 5;
const BLOB_VIEW_MAX: usize = 512 << 10;
/// distinct refs whose indexes stay resident (each ~45k × ~60 B on git.git)
const IDX_CAP: usize = 4;

struct Ent {
    path: Vec<u8>,
    blob: gix::ObjectId,
    mask: u64,
}

struct RefIndex {
    ents: Vec<Ent>,
}

static IDX: OnceLock<Mutex<HashMap<(PathBuf, gix::ObjectId), Arc<RefIndex>>>> = OnceLock::new();

#[derive(serde::Serialize)]
pub struct FileHit {
    pub path: String,
    pub score: i32,
}

#[derive(serde::Serialize)]
pub struct LineHit {
    pub path: String,
    pub line: usize,
    pub text: String,
}

#[derive(serde::Serialize)]
pub struct SearchResp {
    pub took_ms: u128,
    pub files: usize,
    pub shown: usize,
    pub hits: Vec<FileHit>,
    pub matches: Vec<LineHit>,
    pub truncated: bool,
}

#[derive(serde::Serialize)]
pub struct BlobResp {
    pub path: String,
    pub binary: bool,
    pub truncated: bool,
    pub text: String,
}

/// one bit per byte folded into 0..63 — a missing bit proves a term can't
/// match, so non-candidates die on a single AND
fn char_bit(b: u8) -> u64 {
    1 << (b & 63)
}

fn mask_of(s: &[u8]) -> u64 {
    s.iter().fold(0, |m, &b| m | char_bit(b))
}

fn fits(term_mask: u64, path_mask: u64) -> bool {
    term_mask & !path_mask == 0
}

fn fold(b: u8) -> u8 {
    b.to_ascii_lowercase()
}

fn subseq_ci(hay: &[u8], needle: &[u8]) -> bool {
    let mut i = 0;
    for &n in needle {
        loop {
            if i >= hay.len() {
                return false;
            }
            if fold(hay[i]) == n {
                break;
            }
            i += 1;
        }
        i += 1;
    }
    true
}

/// consecutive-run / prefix / whole-name bonuses, length penalty
fn term_score(path: &[u8], t: &[u8]) -> Option<i32> {
    if t.is_empty() {
        return Some(0);
    }
    if !subseq_ci(path, t) {
        return None;
    }
    let mut sc = 10 * t.len() as i32;
    let (mut ti, mut run) = (0usize, 0i32);
    for &b in path {
        if ti < t.len() && fold(b) == t[ti] {
            ti += 1;
            run += 1;
        } else {
            if run >= 2 {
                sc += run * 3;
            }
            run = 0;
        }
    }
    if run >= 2 {
        sc += run * 3;
    }
    if path.len() >= t.len() && (0..t.len()).all(|j| fold(path[j]) == t[j]) {
        sc += 30;
    }
    if path.len() == t.len() && (0..t.len()).all(|j| fold(path[j]) == t[j]) {
        sc += 60;
    }
    Some(sc - (path.len() as i32).min(120) / 3)
}

fn ends_with_ext(path: &[u8], e: &[u8]) -> bool {
    path.len() > e.len() + 1
        && path[path.len() - e.len() - 1] == b'.'
        && path[path.len() - e.len()..]
            .iter()
            .zip(e)
            .all(|(&a, &b)| fold(a) == b)
}

fn contains_ci(hay: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.len() < needle.len() {
        return false;
    }
    let n0 = fold(needle[0]);
    'outer: for i in 0..=hay.len() - needle.len() {
        if fold(hay[i]) != n0 {
            continue;
        }
        for (j, &n) in needle.iter().enumerate().skip(1) {
            if fold(hay[i + j]) != n {
                continue 'outer;
            }
        }
        return true;
    }
    false
}

struct Parsed {
    terms: Vec<Vec<u8>>,
    neg_terms: Vec<Vec<u8>>,
    greps: Vec<Vec<u8>>,
    neg_greps: Vec<Vec<u8>>,
    exts: Vec<Vec<u8>>,
}

/// `word fuzzy path · ext:rs,c · grep:text · -negate`
fn parse(q: &str) -> Parsed {
    let mut p = Parsed {
        terms: Vec::new(),
        neg_terms: Vec::new(),
        greps: Vec::new(),
        neg_greps: Vec::new(),
        exts: Vec::new(),
    };
    for raw in q.split_whitespace() {
        let (neg, w) = match raw.strip_prefix('-') {
            Some(r) if !r.is_empty() => (true, r),
            _ => (false, raw),
        };
        if let Some(v) = w.strip_prefix("ext:") {
            let e = v.trim_start_matches('.').to_ascii_lowercase().into_bytes();
            if !e.is_empty() {
                p.exts.push(e);
            }
        } else if let Some(v) = w.strip_prefix("grep:").or_else(|| w.strip_prefix("content:")) {
            let g = v.to_ascii_lowercase().into_bytes();
            if !g.is_empty() {
                if neg {
                    p.neg_greps.push(g);
                } else {
                    p.greps.push(g);
                }
            }
        } else {
            let t = w.to_ascii_lowercase().into_bytes();
            if !t.is_empty() {
                if neg {
                    p.neg_terms.push(t);
                } else {
                    p.terms.push(t);
                }
            }
        }
    }
    p
}

/// every blob under `commit`, flat and sorted; trees only — no blob loads,
/// so building git.git's ~45k-entry index is a tree walk, nothing more
fn index_of(mirror: &Path, repo: &gix::Repository, commit: gix::ObjectId) -> Result<Arc<RefIndex>> {
    let cache = IDX.get_or_init(Default::default);
    let key = (mirror.to_path_buf(), commit);
    if let Ok(g) = cache.lock() {
        if let Some(ix) = g.get(&key) {
            return Ok(ix.clone());
        }
    }
    let root = repo
        .find_object(commit)?
        .try_into_commit()
        .context("search target is not a commit")?
        .tree()?;
    let mut ents = Vec::new();
    let mut stack: Vec<(gix::Tree<'_>, Vec<u8>)> = vec![(root, Vec::new())];
    while let Some((tree, prefix)) = stack.pop() {
        for entry in tree.iter() {
            let entry = entry?;
            let mode = entry.mode();
            let mut path = prefix.clone();
            if !path.is_empty() {
                path.push(b'/');
            }
            path.extend_from_slice(entry.filename().as_bytes());
            if mode.is_tree() {
                stack.push((entry.object()?.try_into_tree()?, path));
            } else if mode.is_blob() {
                let mask = mask_of(&path);
                ents.push(Ent {
                    path,
                    blob: entry.object_id(),
                    mask,
                });
            }
            // gitlinks have no content of their own — skipped
        }
    }
    ents.sort_by(|a, b| a.path.cmp(&b.path));
    let ix = Arc::new(RefIndex { ents });
    if let Ok(mut g) = cache.lock() {
        if g.len() >= IDX_CAP {
            g.clear();
        }
        g.insert(key, ix.clone());
    }
    Ok(ix)
}

pub fn search(mirror: &PathBuf, refspec: &str, q: &str) -> Result<SearchResp> {
    let t0 = Instant::now();
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let commit = crate::diff::commit_of(&repo, refspec)?.id().detach();
    let ix = index_of(mirror, &repo, commit)?;
    let p = parse(q);

    let term_masks: Vec<u64> = p.terms.iter().map(|t| mask_of(t)).collect();
    let neg_masks: Vec<u64> = p.neg_terms.iter().map(|t| mask_of(t)).collect();
    let mut hits: Vec<(i32, &Ent)> = Vec::new();
    let mut cand: Vec<&Ent> = Vec::new();
    for e in &ix.ents {
        if !p.exts.is_empty() && !p.exts.iter().any(|x| ends_with_ext(&e.path, x)) {
            continue;
        }
        if p.neg_terms
            .iter()
            .zip(&neg_masks)
            .any(|(nt, &m)| fits(m, e.mask) && subseq_ci(&e.path, nt))
        {
            continue;
        }
        if !p.terms.is_empty() {
            if !term_masks.iter().all(|&m| fits(m, e.mask)) {
                continue;
            }
            let mut sc = 0i32;
            let mut ok = true;
            for t in &p.terms {
                match term_score(&e.path, t) {
                    Some(s) => sc += s,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if !ok {
                continue;
            }
            hits.push((sc, e));
        }
        cand.push(e);
    }

    hits.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.path.len().cmp(&b.1.path.len())));
    let files_total = hits.len();
    let files: Vec<FileHit> = hits
        .into_iter()
        .take(MAX_FILES_OUT)
        .map(|(score, e)| FileHit {
            path: String::from_utf8_lossy(&e.path).into_owned(),
            score,
        })
        .collect();

    let mut matches = Vec::new();
    let mut truncated = false;
    if !p.greps.is_empty() || !p.neg_greps.is_empty() {
        let deadline = t0 + GREP_BUDGET;
        'outer: for e in cand.iter() {
            if Instant::now() > deadline {
                truncated = true;
                break;
            }
            let Ok(blob) = repo.find_blob(e.blob) else {
                continue;
            };
            let data: &[u8] = blob.data.as_bytes();
            if data.len() as u64 > MAX_FILE || data.iter().take(8192).any(|&b| b == 0) {
                continue;
            }
            if p.neg_greps.iter().any(|g| contains_ci(data, g)) {
                continue;
            }
            let mut per = 0usize;
            for (li, line) in data.split(|&b| b == b'\n').enumerate() {
                if !p.greps.iter().all(|g| contains_ci(line, g)) {
                    continue;
                }
                if per < MAX_HITS_PER_FILE && matches.len() < MAX_HITS_OUT {
                    let text = String::from_utf8_lossy(line);
                    let text = text.trim();
                    let text: String = if text.len() > 240 {
                        let mut end = 240;
                        while end > 0 && !text.is_char_boundary(end) {
                            end -= 1;
                        }
                        format!("{}…", &text[..end])
                    } else {
                        text.to_string()
                    };
                    matches.push(LineHit {
                        path: String::from_utf8_lossy(&e.path).into_owned(),
                        line: li + 1,
                        text,
                    });
                    per += 1;
                }
                if matches.len() >= MAX_HITS_OUT {
                    truncated = true;
                    break 'outer;
                }
            }
        }
    }

    Ok(SearchResp {
        took_ms: t0.elapsed().as_millis(),
        files: files_total,
        shown: files.len(),
        hits: files,
        matches,
        truncated,
    })
}

pub fn blob_text(mirror: &PathBuf, refspec: &str, path: &str) -> Result<BlobResp> {
    let tsr = crate::repo::get(mirror)?;
    let repo = crate::repo::handle(&tsr);
    let commit = crate::diff::commit_of(&repo, refspec)?;
    let tree = commit.tree()?;
    let Some(entry) = tree.lookup_entry_by_path(path)? else {
        return Err(crate::diff::not_found(format!("path not found: {path}")));
    };
    let mode = entry.mode();
    if mode.is_tree() {
        return Err(crate::diff::not_found(format!("not a file: {path}")));
    }
    if mode.is_commit() {
        return Ok(BlobResp {
            path: path.to_string(),
            binary: false,
            truncated: false,
            text: String::new(),
        });
    }
    let blob = entry.object()?.try_into_blob()?;
    let data: &[u8] = blob.data.as_bytes();
    let binary = data.iter().take(8192).any(|&b| b == 0);
    if binary {
        return Ok(BlobResp {
            path: path.to_string(),
            binary: true,
            truncated: false,
            text: String::new(),
        });
    }
    let s = String::from_utf8_lossy(data);
    let (text, truncated) = if s.len() > BLOB_VIEW_MAX {
        let mut end = BLOB_VIEW_MAX;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        (s[..end].to_string(), true)
    } else {
        (s.into_owned(), false)
    };
    Ok(BlobResp {
        path: path.to_string(),
        binary: false,
        truncated,
        text,
    })
}

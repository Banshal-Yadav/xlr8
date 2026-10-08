use gix::object::tree::diff::ChangeDetached as Change;
use gix::object::tree::EntryKind;
use gix::ObjectId;
use std::collections::HashMap;
use std::sync::Arc;

const PERCENTAGE: f32 = 0.5;
const RENAME_LIMIT: usize = 32767;

struct Item {
    change_idx: usize,
    dest: bool,
    id: ObjectId,
    mode: gix::object::tree::EntryMode,
    path: gix::bstr::BString,
    emitted: bool,
}

fn pairable(mode: gix::object::tree::EntryMode) -> bool {
    matches!(
        mode.kind(),
        EntryKind::Blob | EntryKind::BlobExecutable | EntryKind::Link
    )
}

fn compatible(a: gix::object::tree::EntryMode, b: gix::object::tree::EntryMode) -> bool {
    matches!(
        (a.kind(), b.kind()),
        (EntryKind::Blob | EntryKind::BlobExecutable, EntryKind::Blob | EntryKind::BlobExecutable)
            | (EntryKind::Link, EntryKind::Link)
    )
}

fn load(
    mirror: &std::path::Path,
    repo: &gix::Repository,
    id: ObjectId,
) -> Option<std::sync::Arc<[u8]>> {
    if let Some(b) = super::repo::blob_get(mirror, id) {
        return Some(b);
    }
    let mut obj = repo.find_object(id).ok()?;
    if !obj.kind.is_blob() {
        return None;
    }
    let data: std::sync::Arc<[u8]> = std::mem::take(&mut obj.data).into();
    super::repo::blob_put(mirror, id, data.clone());
    Some(data)
}

fn dir_of(path: &[u8]) -> &[u8] {
    match path.iter().rposition(|&b| b == b'/') {
        Some(p) => &path[..p],
        None => b"",
    }
}

/// Score every (dest, src) pair for one slice of dests; one thread's worth of
/// work. Loads blobs up front (cache fills on first load), interns each src
/// once, skips binaries and sub-50% size pairs. When the full cross product
/// exceeds RENAME_LIMIT, `same_dir_only` restricts pairs to one directory —
/// git's own windowing fallback for oversized rename detection.
fn score_part(
    mirror: &std::path::Path,
    tsr: &Arc<gix::ThreadSafeRepository>,
    part: Vec<(usize, ObjectId, gix::object::tree::EntryMode)>,
    srcs: &[(usize, ObjectId, gix::object::tree::EntryMode)],
    dirs: &[gix::bstr::BString],
    same_dir_only: bool,
) -> Vec<(usize, usize, f32)> {
    let repo = super::repo::handle(tsr);

    let mut blobs: HashMap<ObjectId, (std::sync::Arc<[u8]>, bool)> = HashMap::new();
    for (_, did, _) in &part {
        if !blobs.contains_key(did) {
            if let Some(b) = load(mirror, &repo, *did) {
                let bin = super::diff::has_nul(&b);
                blobs.insert(*did, (b, bin));
            }
        }
    }
    for (_, sid, _) in srcs.iter() {
        if !blobs.contains_key(sid) {
            if let Some(b) = load(mirror, &repo, *sid) {
                let bin = super::diff::has_nul(&b);
                blobs.insert(*sid, (b, bin));
            }
        }
    }

    let mut input: imara_diff::intern::InternedInput<&[u8]> = Default::default();
    let mut src_ids: HashMap<usize, Vec<imara_diff::intern::Token>> = HashMap::new();
    let mut out: Vec<(usize, usize, f32)> = Vec::new();

    for (di, did, dmode) in &part {
        if !dmode.is_blob() {
            continue;
        }
        let Some((dbytes, dbin)) = blobs.get(did) else {
            continue;
        };
        if *dbin {
            continue;
        }
        input.update_before(imara_diff::sources::byte_lines_with_terminator(dbytes));
        for (si, sid, smode) in srcs.iter() {
            if !compatible(*smode, *dmode) {
                continue;
            }
            if same_dir_only && dirs[*si] != dirs[*di] {
                continue;
            }
            let Some((sbytes, sbin)) = blobs.get(sid) else {
                continue;
            };
            if *sbin {
                continue;
            }
            // similarity <= min/max, so pairs smaller than 50% size can never match
            if sbytes.len().min(dbytes.len()) * 2 < sbytes.len().max(dbytes.len()) {
                continue;
            }
            if let Some(ids) = src_ids.get(si) {
                input.after.clear();
                input.after.extend_from_slice(ids);
            } else {
                input.update_after(imara_diff::sources::byte_lines_with_terminator(sbytes));
                src_ids.insert(*si, input.after.clone());
            }
            let mut removed = 0usize;
            imara_diff::diff(
                imara_diff::Algorithm::Myers,
                &input,
                |before: std::ops::Range<u32>, _after: std::ops::Range<u32>| {
                    removed += (before.start as usize..before.end as usize)
                        .map(|i| input.interner[input.before[i]].len())
                        .sum::<usize>();
                },
            );
            let sim = (dbytes.len() - removed) as f32 / dbytes.len().max(sbytes.len()) as f32;
            if sim >= PERCENTAGE {
                out.push((*di, *si, sim));
            }
        }
    }
    out
}

pub fn detect(
    tsr: &Arc<gix::ThreadSafeRepository>,
    mirror: &std::path::Path,
    changes: &mut Vec<(usize, Change)>,
) -> anyhow::Result<()> {
    let t0 = std::time::Instant::now();

    let mut items: Vec<Item> = Vec::new();
    for (i, (_, change)) in changes.iter().enumerate() {
        match change {
            Change::Addition {
                location,
                entry_mode,
                id,
                ..
            } => {
                if pairable(*entry_mode) && !id.is_empty_blob() {
                    items.push(Item {
                        change_idx: i,
                        dest: true,
                        id: *id,
                        mode: *entry_mode,
                        path: location.clone(),
                        emitted: false,
                    });
                }
            }
            Change::Deletion {
                location,
                entry_mode,
                id,
                ..
            } => {
                if pairable(*entry_mode) && !id.is_empty_blob() {
                    items.push(Item {
                        change_idx: i,
                        dest: false,
                        id: *id,
                        mode: *entry_mode,
                        path: location.clone(),
                        emitted: false,
                    });
                }
            }
            _ => {}
        }
    }
    items.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.path.cmp(&b.path)));

    let mut pairs: Vec<(usize, usize)> = Vec::new();
    let mut dest_ofs = 0;
    while let Some(di) = (dest_ofs..items.len()).find(|&i| !items[i].emitted && items[i].dest) {
        dest_ofs = di + 1;
        if let Some(si) = (0..items.len()).find(|&si| {
            si != di
                && !items[si].emitted
                && !items[si].dest
                && items[si].id == items[di].id
                && compatible(items[si].mode, items[di].mode)
        }) {
            items[di].emitted = true;
            items[si].emitted = true;
            pairs.push((di, si));
        }
    }

    let srcs: Vec<(usize, ObjectId, gix::object::tree::EntryMode)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| !it.emitted && !it.dest)
        .map(|(i, it)| (i, it.id, it.mode))
        .collect();
    let dests: Vec<(usize, ObjectId, gix::object::tree::EntryMode)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| !it.emitted && it.dest)
        .map(|(i, it)| (i, it.id, it.mode))
        .collect();

    // dir-windowed fallback: over the renameLimit git scores only same-directory
    // pairs (its diffcore-rename windows) instead of skipping detection entirely
    let n_cross = srcs.len() * dests.len();
    let same_dir_only = n_cross > RENAME_LIMIT;
    if !dests.is_empty() && !srcs.is_empty() {
        let srcs = Arc::new(srcs);
        let dirs: Arc<Vec<gix::bstr::BString>> =
            Arc::new(items.iter().map(|it| dir_of(&it.path).into()).collect());
        // rename scoring caps at 8: A/B on 8c/16t — 16 threads regressed
        // 60→76 ms (memory-bound blob loads, SMT contention), while stats
        // fill wants every thread (stress 88→54 ms). Split caps.
        let n_threads = crate::repo::n_threads().min(8).min(dests.len());

        let mut fuzzy: Vec<(usize, usize, f32)> = Vec::new();
        // thread spawn costs more than scoring a handful of pairs
        if n_threads <= 1 || n_cross <= 64 {
            fuzzy.extend(score_part(mirror, tsr, dests, &srcs, &dirs, same_dir_only));
        } else {
            let chunk = dests.len().div_ceil(n_threads).max(1);
            let mut handles = Vec::with_capacity(n_threads);
            let mut rest = dests.into_iter();
            loop {
                let part: Vec<(usize, ObjectId, gix::object::tree::EntryMode)> =
                    rest.by_ref().take(chunk).collect();
                if part.is_empty() {
                    break;
                }
                let tsr = tsr.clone();
                let srcs = srcs.clone();
                let dirs = dirs.clone();
                let mirror = mirror.to_path_buf();
                handles.push(std::thread::spawn(move || {
                    score_part(&mirror, &tsr, part, &srcs, &dirs, same_dir_only)
                }));
            }
            for h in handles {
                let part = h
                    .join()
                    .map_err(|_| anyhow::anyhow!("rename worker panicked"))?;
                fuzzy.extend(part);
            }
        }
        // highest similarity claims contested sources first (git's
        // diffcore_rename order), ties broken by item index
        fuzzy.sort_by(|a, b| {
            b.2.partial_cmp(&a.2)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
                .then_with(|| a.1.cmp(&b.1))
        });
        for (di, si, _) in fuzzy {
            if !items[di].emitted && !items[si].emitted {
                items[di].emitted = true;
                items[si].emitted = true;
                pairs.push((di, si));
            }
        }
    }

    if !pairs.is_empty() {
        let mut drop_src: Vec<usize> = pairs.iter().map(|&(_, si)| items[si].change_idx).collect();
        drop_src.sort_unstable();
        let mut dest_of_change: HashMap<usize, (usize, usize)> = HashMap::new();
        for (di, si) in &pairs {
            dest_of_change.insert(items[*di].change_idx, (*di, *si));
        }

        let mut out: Vec<(usize, Change)> = Vec::with_capacity(changes.len() - drop_src.len());
        for (pos, (orig, change)) in std::mem::take(changes).into_iter().enumerate() {
            if drop_src.binary_search(&pos).is_ok() {
                continue;
            }
            if let Some((di, si)) = dest_of_change.get(&pos) {
                out.push((
                    orig,
                    Change::Rewrite {
                        location: items[*di].path.clone(),
                        entry_mode: items[*di].mode,
                        source_location: items[*si].path.clone(),
                        source_entry_mode: items[*si].mode,
                        source_id: items[*si].id,
                        id: items[*di].id,
                        source_relation: None,
                        relation: None,
                        diff: None,
                        copy: false,
                    },
                ));
            } else {
                out.push((orig, change));
            }
        }
        *changes = out;
    }

    crate::debug!(
        "renames: {} pairs from {} candidates in {:?}",
        pairs.len(),
        items.len(),
        t0.elapsed()
    );
    Ok(())
}

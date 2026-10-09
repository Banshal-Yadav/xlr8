use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

static REPOS: OnceLock<RwLock<RepoMap>> = OnceLock::new();
type RepoMap = std::collections::HashMap<PathBuf, Arc<gix::ThreadSafeRepository>>;

/// Per-mirror blob memo: warm requests skip pack lookup + inflate entirely.
///
/// Sharded 16-way (power of two → mask in `shard_idx`): each shard locks
/// itself and overflow clears only that shard. Evicted wholesale for a mirror
/// on its pull (`invalidate`).
const BLOB_CACHE_CAP: usize = 64 << 20;
const BLOB_SHARDS: usize = 16;
struct BlobShard {
    map: HashMap<PathBuf, HashMap<gix::ObjectId, Arc<[u8]>>>,
    bytes: usize,
}
static BLOBS: OnceLock<[Mutex<BlobShard>; BLOB_SHARDS]> = OnceLock::new();

fn blobs() -> &'static [Mutex<BlobShard>; BLOB_SHARDS] {
    BLOBS.get_or_init(|| {
        std::array::from_fn(|_| {
            Mutex::new(BlobShard {
                map: HashMap::new(),
                bytes: 0,
            })
        })
    })
}

fn shard_idx(path: &Path, id: &gix::ObjectId) -> usize {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    id.hash(&mut h);
    (h.finish() as usize) & (BLOB_SHARDS - 1)
}

pub fn blob_get(path: &Path, id: gix::ObjectId) -> Option<Arc<[u8]>> {
    let s = blobs()[shard_idx(path, &id)].lock().ok()?;
    s.map.get(path)?.get(&id).cloned()
}

pub fn blob_put(path: &Path, id: gix::ObjectId, data: Arc<[u8]>) {
    let Ok(mut s) = blobs()[shard_idx(path, &id)].lock() else {
        return;
    };
    let cap = BLOB_CACHE_CAP / BLOB_SHARDS;
    if s.bytes + data.len() > cap {
        s.map.clear();
        s.bytes = 0;
    }
    s.bytes += data.len();
    s.map.entry(path.to_path_buf()).or_default().insert(id, data);
}

pub fn get(path: &Path) -> anyhow::Result<Arc<gix::ThreadSafeRepository>> {
    let map = REPOS.get_or_init(Default::default);
    if let Ok(m) = map.read() {
        if let Some(r) = m.get(path) {
            return Ok(r.clone());
        }
    }
    let repo = Arc::new(
        gix::open(path)
            .map_err(|e| crate::diff::NotFound(format!("cannot open {}: {e}", path.display())))?
            .into_sync(),
    );
    if let Ok(mut m) = map.write() {
        m.insert(path.to_path_buf(), repo.clone());
    }
    Ok(repo)
}

/// Thread-local handle with a small object cache: repeat reads of the same
/// commit/tree within one request (commits walk, `spec^` peels) skip inflate.
pub fn handle(tsr: &gix::ThreadSafeRepository) -> gix::Repository {
    let mut repo = tsr.to_thread_local();
    repo.object_cache_size_if_unset(1 << 20);
    repo
}

/// Worker cap for parallel stats/scoring: CPU count, probed once
/// (`available_parallelism` is a syscall per call), clamped to 16.
/// Rename scoring tightens this further — see rewrite.rs.
/// Override: `XLR8_THREADS=N`.
pub fn n_threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        if let Ok(v) = std::env::var("XLR8_THREADS") {
            if let Ok(n) = v.parse::<usize>() {
                return n.clamp(1, 64);
            }
        }
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 16)
    })
}

pub fn invalidate(path: &Path) {
    if let Some(map) = REPOS.get() {
        if let Ok(mut m) = map.write() {
            m.remove(path);
        }
    }
    for shard in blobs() {
        if let Ok(mut s) = shard.lock() {
            if let Some(v) = s.map.remove(path) {
                s.bytes -= v.values().map(|v| v.len()).sum::<usize>();
            }
        }
    }
    crate::diff::clear_ref_cache(path);
    crate::server::clear_resp_cache(path);
}

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

static REPOS: OnceLock<RwLock<RepoMap>> = OnceLock::new();
type RepoMap = std::collections::HashMap<PathBuf, Arc<gix::ThreadSafeRepository>>;

/// Per-mirror blob memo: warm requests skip pack lookup + inflate entirely.
/// Cleared wholesale on overflow or on that mirror's pull.
const BLOB_CACHE_CAP: usize = 64 << 20;
struct BlobCache {
    map: HashMap<PathBuf, HashMap<gix::ObjectId, Arc<[u8]>>>,
    bytes: usize,
}
static BLOBS: OnceLock<Mutex<BlobCache>> = OnceLock::new();

fn blobs() -> &'static Mutex<BlobCache> {
    BLOBS.get_or_init(|| {
        Mutex::new(BlobCache {
            map: HashMap::new(),
            bytes: 0,
        })
    })
}

pub fn blob_get(path: &Path, id: gix::ObjectId) -> Option<Arc<[u8]>> {
    blobs().lock().ok()?.map.get(path)?.get(&id).cloned()
}

pub fn blob_put(path: &Path, id: gix::ObjectId, data: Arc<[u8]>) {
    let Ok(mut g) = blobs().lock() else {
        return;
    };
    if g.bytes + data.len() > BLOB_CACHE_CAP {
        g.map.clear();
        g.bytes = 0;
    }
    g.bytes += data.len();
    g.map.entry(path.to_path_buf()).or_default().insert(id, data);
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

/// CPU count, probed once — `available_parallelism` is a syscall per call.
pub fn n_threads() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(4)
            .clamp(1, 8)
    })
}

pub fn invalidate(path: &Path) {
    if let Some(map) = REPOS.get() {
        if let Ok(mut m) = map.write() {
            m.remove(path);
        }
    }
    if let Ok(mut g) = blobs().lock() {
        if let Some(v) = g.map.remove(path) {
            g.bytes -= v.values().map(|v| v.len()).sum::<usize>();
        }
    }
    crate::diff::clear_ref_cache(path);
    crate::server::clear_resp_cache(path);
}

use crate::{diff, mirror, search, AppState};
use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{Html, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static REQ: AtomicU64 = AtomicU64::new(0);
static START: OnceLock<Instant> = OnceLock::new();
/// epoch second of the last successful sync (boot counts); 0 = unknown
static SYNCED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn note_sync() {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    SYNCED.store(s.max(1), Ordering::Relaxed);
}

/// Serialized JSON per (mirror, request). The mirror is immutable between
/// pulls, so a repeat compare/file/commits/refs request is served straight
/// from memory — no walk, no diff, no serde.
const RESP_CAP: usize = 32 << 20;
struct RespCache {
    map: HashMap<String, Bytes>,
    bytes: usize,
}
static RESP: OnceLock<Mutex<RespCache>> = OnceLock::new();

fn resp_key(path: &std::path::Path, rest: &str) -> String {
    let mut k = path.to_string_lossy().into_owned();
    k.push('\0');
    k.push_str(rest);
    k
}

fn resp_get(key: &str) -> Option<Bytes> {
    RESP.get()?.lock().ok()?.map.get(key).cloned()
}

fn resp_put(key: String, body: Vec<u8>) -> Bytes {
    let bytes = Bytes::from(body);
    let c = RESP.get_or_init(|| {
        Mutex::new(RespCache {
            map: HashMap::new(),
            bytes: 0,
        })
    });
    if bytes.len() > RESP_CAP {
        return bytes;
    }
    let Ok(mut g) = c.lock() else {
        return bytes;
    };
    if g.bytes + bytes.len() > RESP_CAP {
        g.map.clear();
        g.bytes = 0;
    }
    g.bytes += bytes.len();
    g.map.insert(key, bytes.clone());
    bytes
}

/// Called on sync: the mirror's objects moved, every cached body is stale.
pub(crate) fn clear_resp_cache(path: &std::path::Path) {
    let Some(c) = RESP.get() else {
        return;
    };
    let Ok(mut g) = c.lock() else {
        return;
    };
    let prefix = format!("{}\0", path.to_string_lossy());
    g.map.retain(|k, _| !k.starts_with(&prefix));
    g.bytes = g.map.values().map(|v| v.len()).sum();
}

fn json_body(bytes: Bytes) -> Response {
    Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(bytes))
        .unwrap_or_else(|_| Response::new(Body::empty()))
}

pub fn router(state: AppState) -> Router {
    START.get_or_init(Instant::now);
    Router::new()
        .route("/", get(index))
        .route("/api/repos", get(list_repos))
        .route("/api/stats", get(stats))
        .route("/api/sync", post(sync_repo))
        .route("/api/{owner}/{repo}/refs", get(refs))
        .route("/api/{owner}/{repo}/commits", get(commits))
        .route("/api/{owner}/{repo}/diff", get(diff_summary))
        .route("/api/{owner}/{repo}/file", get(file_diff))
        .route("/api/{owner}/{repo}/search", get(search))
        .route("/api/{owner}/{repo}/blob", get(blob))
        .route("/api/{owner}/{repo}/pulls", get(pulls))
        .layer(middleware::from_fn(count_requests))
        .with_state(state)
}

/// One relaxed atomic add per request — ~1 ns on a 700 µs–4 ms endpoint.
async fn count_requests(req: Request, next: Next) -> Response {
    REQ.fetch_add(1, Ordering::Relaxed);
    next.run(req).await
}

#[derive(Serialize)]
struct StatsResp {
    rss: u64,
    cpu_ms: u64,
    uptime_s: u64,
    reqs: u64,
    cache_bytes: usize,
    cache_entries: usize,
    /// seconds since the last successful sync; 0 = unknown
    synced_s: u64,
}

/// Reads /proc only when hit (frontend throttles to ≥2 s) — no hot path
/// touches it, and nothing is precomputed per request.
async fn stats() -> Json<StatsResp> {
    let mut rss = 0u64;
    if let Ok(s) = std::fs::read_to_string("/proc/self/status") {
        for line in s.lines() {
            if let Some(v) = line.strip_prefix("VmRSS:") {
                rss = v
                    .trim()
                    .trim_end_matches("kB")
                    .trim()
                    .parse::<u64>()
                    .unwrap_or(0)
                    * 1024;
                break;
            }
        }
    }
    let mut cpu_ms = 0u64;
    if let Ok(s) = std::fs::read_to_string("/proc/self/stat") {
        // comm field may contain spaces/parens — fields after the last ')'
        if let Some(rest) = s.rfind(')').map(|i| &s[i + 1..]) {
            let f: Vec<&str> = rest.split_whitespace().collect();
            if f.len() >= 13 {
                // utime = field 14, stime = 15 → indices 11, 12 after state;
                // CLK_TCK = 100 on Linux/Android → 10 ms per tick
                let ut: u64 = f[11].parse().unwrap_or(0);
                let st: u64 = f[12].parse().unwrap_or(0);
                cpu_ms = (ut + st) * 10;
            }
        }
    }
    let (cache_bytes, cache_entries) = RESP
        .get()
        .and_then(|c| c.lock().ok().map(|g| (g.bytes, g.map.len())))
        .unwrap_or((0, 0));
    let synced_s = SYNCED.load(Ordering::Relaxed);
    let synced_s = if synced_s == 0 {
        0
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().saturating_sub(synced_s))
            .unwrap_or(0)
    };
    Json(StatsResp {
        rss,
        cpu_ms,
        uptime_s: START.get().map(|t| t.elapsed().as_secs()).unwrap_or(0),
        reqs: REQ.load(Ordering::Relaxed),
        cache_bytes,
        cache_entries,
        synced_s,
    })
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../assets/index.html"))
}

#[derive(Serialize)]
struct ReposResp {
    repos: Vec<String>,
}

async fn list_repos() -> Json<ReposResp> {
    Json(ReposResp {
        repos: mirror::cached_repos()
            .iter()
            .map(|r| r.canonical())
            .collect(),
    })
}

#[derive(Deserialize)]
struct SyncReq {
    repo: String,
}

#[derive(Serialize)]
struct SyncResp {
    ok: bool,
}

async fn sync_repo(Json(req): Json<SyncReq>) -> Result<Json<SyncResp>, (StatusCode, String)> {
    let repo = mirror::RepoRef::parse(&req.repo).map_err(api_err)?;
    let path = tokio::task::spawn_blocking(move || mirror::sync(&repo))
        .await
        .map_err(|e| api_err(e.into()))?
        .map_err(api_err)?;
    crate::repo::invalidate(&path);
    note_sync();
    Ok(Json(SyncResp { ok: true }))
}

#[derive(Serialize)]
struct RefsResp {
    refs: Vec<diff::RefInfo>,
}

async fn refs(
    Path((owner, repo)): Path<(String, String)>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef {
        owner,
        name: repo,
    };
    let path = mirror::mirror_path(&r);
    let key = resp_key(&path, "refs");
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let body = tokio::task::spawn_blocking(move || {
        let refs = diff::refs(&path)?;
        serde_json::to_vec(&RefsResp { refs }).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Deserialize)]
struct CommitsQuery {
    limit: Option<usize>,
    r#ref: Option<String>,
    /// `stats=1` fills +/− per commit (second cached pass; list without it
    /// stays a pure commit walk). String, not bool — axum rejects `1` as bool.
    stats: Option<String>,
    /// 1-based page; page N walks past (N-1)*limit commits
    page: Option<usize>,
    /// `count=1` → {"count": N} total reachable commits (separate cached
    /// pass; list never blocks on it)
    count: Option<String>,
}

#[derive(Serialize)]
struct CommitsResp {
    commits: Vec<diff::CommitInfo>,
}

#[derive(Serialize)]
struct CountResp {
    count: usize,
}

async fn commits(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<CommitsQuery>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let refspec = q.r#ref.clone();
    if q.count.as_deref() == Some("1") {
        let key = resp_key(
            &path,
            &format!("commits_count\0{}", refspec.as_deref().unwrap_or("")),
        );
        if let Some(b) = resp_get(&key) {
            return Ok(json_body(b));
        }
        let body = tokio::task::spawn_blocking(move || {
            let count = diff::commit_count(&path, refspec.as_deref())?;
            serde_json::to_vec(&CountResp { count }).map_err(anyhow::Error::from)
        })
        .await
        .map_err(|e| api_err(e.into()))?
        .map_err(api_err)?;
        return Ok(json_body(resp_put(key, body)));
    }
    let limit = q.limit.unwrap_or(50).min(500);
    let want_stats = q.stats.as_deref() == Some("1");
    let page = q.page.unwrap_or(1).max(1);
    let offset = page.saturating_sub(1).saturating_mul(limit);
    let key = resp_key(
        &path,
        &format!(
            "commits\0{limit}\0{}\0{want_stats}\0{page}",
            refspec.as_deref().unwrap_or("")
        ),
    );
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let body = tokio::task::spawn_blocking(move || {
        let list = diff::commits(&path, limit, refspec.as_deref(), want_stats, offset)?;
        serde_json::to_vec(&CommitsResp { commits: list }).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Deserialize)]
struct RangeQuery {
    base: String,
    head: String,
}

async fn diff_summary(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<RangeQuery>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let key = resp_key(&path, &format!("diff\0{}\0{}", q.base, q.head));
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let (base, head) = (q.base.clone(), q.head.clone());
    let body = tokio::task::spawn_blocking(move || {
        let summary = diff::compare(&path, &base, &head)?;
        serde_json::to_vec(&summary).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Deserialize)]
struct FileQuery {
    base: String,
    head: String,
    path: String,
    old_path: Option<String>,
}

async fn file_diff(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<FileQuery>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let key = resp_key(
        &path,
        &format!(
            "file\0{}\0{}\0{}\0{}",
            q.base,
            q.head,
            q.path,
            q.old_path.as_deref().unwrap_or("")
        ),
    );
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let (base, head, p, op) = (q.base.clone(), q.head.clone(), q.path.clone(), q.old_path);
    let body = tokio::task::spawn_blocking(move || {
        let fd = diff::file_diff(&path, &base, &head, &p, op.as_deref())?;
        serde_json::to_vec(&fd).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Deserialize)]
struct SearchQuery {
    r#ref: Option<String>,
    q: String,
}

async fn search(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<SearchQuery>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let refspec = q.r#ref.clone().unwrap_or_else(|| "HEAD".to_string());
    let key = resp_key(&path, &format!("search\0{refspec}\0{}", q.q));
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let query = q.q.clone();
    let body = tokio::task::spawn_blocking(move || {
        let sr = search::search(&path, &refspec, &query)?;
        serde_json::to_vec(&sr).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Deserialize)]
struct BlobQuery {
    r#ref: Option<String>,
    path: String,
}

async fn blob(
    Path((owner, repo)): Path<(String, String)>,
    Query(q): Query<BlobQuery>,
) -> Result<Response, (StatusCode, String)> {
    let r = mirror::RepoRef { owner, name: repo };
    let path = mirror::mirror_path(&r);
    let refspec = q.r#ref.clone().unwrap_or_else(|| "HEAD".to_string());
    let key = resp_key(&path, &format!("blob\0{refspec}\0{}", q.path));
    if let Some(b) = resp_get(&key) {
        return Ok(json_body(b));
    }
    let p = q.path.clone();
    let body = tokio::task::spawn_blocking(move || {
        let br = search::blob_text(&path, &refspec, &p)?;
        serde_json::to_vec(&br).map_err(anyhow::Error::from)
    })
    .await
    .map_err(|e| api_err(e.into()))?
    .map_err(api_err)?;
    Ok(json_body(resp_put(key, body)))
}

#[derive(Serialize)]
struct PullsResp {
    pulls: Vec<crate::github::PullRequest>,
}

async fn pulls(
    Path((owner, repo)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<PullsResp>, (StatusCode, String)> {
    let list = tokio::task::spawn_blocking(move || state.gh.pulls(&owner, &repo))
        .await
        .map_err(|e| api_err(e.into()))?
        .map_err(api_err)?;
    Ok(Json(PullsResp { pulls: list }))
}

fn api_err(e: anyhow::Error) -> (StatusCode, String) {
    let status = if e.downcast_ref::<diff::NotFound>().is_some() {
        StatusCode::NOT_FOUND
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    };
    (status, e.to_string())
}

pub async fn serve(state: AppState, port: Option<u16>) -> anyhow::Result<SocketAddr> {
    use axum::serve::{Listener, ListenerExt};
    let addr = SocketAddr::from(([127, 0, 0, 1], port.unwrap_or(0)));
    let listener = tokio::net::TcpListener::bind(addr)
        .await?
        .tap_io(|s| {
            let _ = s.set_nodelay(true);
        });
    let local = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router(state)).await;
    });
    Ok(local)
}

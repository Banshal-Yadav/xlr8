use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use std::process::Command;

#[derive(Clone)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

impl RepoRef {
    pub fn parse(input: &str) -> Result<Self> {
        let input = input.trim().trim_end_matches('/');
        let rest = input
            .strip_prefix("https://github.com/")
            .or_else(|| input.strip_prefix("http://github.com/"))
            .or_else(|| input.strip_prefix("git@github.com:"))
            .or_else(|| input.strip_prefix("github.com/"))
            .unwrap_or(input);
        let rest = rest.strip_suffix(".git").unwrap_or(rest);
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
            bail!("cannot parse repo from `{input}` (expected owner/repo or a GitHub URL)");
        }
        Ok(Self {
            owner: parts[0].to_string(),
            name: parts[1].to_string(),
        })
    }

    pub fn slug(&self) -> String {
        format!("{}_{}", self.owner, self.name)
    }

    pub fn canonical(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    pub fn https_url(&self) -> String {
        format!("https://github.com/{}/{}", self.owner, self.name)
    }
}

pub fn mirror_path(repo: &RepoRef) -> PathBuf {
    cache_dir()
        .join("mirrors")
        .join(format!("{}.git", repo.slug()))
}

/// Resolved once: env lookups + joins on every API hit add up.
pub fn cache_dir() -> &'static PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(compute_cache_dir)
}

fn compute_cache_dir() -> PathBuf {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            #[cfg(windows)]
            {
                std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
            }
            #[cfg(not(windows))]
            {
                None
            }
        })
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("."));
            #[cfg(target_os = "macos")]
            {
                home.join("Library").join("Caches")
            }
            #[cfg(all(not(windows), not(target_os = "macos")))]
            {
                home.join(".cache")
            }
            #[cfg(windows)]
            {
                home.join("AppData").join("Local")
            }
        });
    base.join("xlr8")
}

/// repos present in the local mirror cache, parsed back from `owner_name.git`
pub fn cached_repos() -> Vec<RepoRef> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(cache_dir().join("mirrors")) {
        for e in rd.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if let Some(slug) = file.strip_suffix(".git") {
                let parts: Vec<&str> = slug.splitn(2, '_').collect();
                if parts.len() == 2 {
                    out.push(RepoRef {
                        owner: parts[0].to_string(),
                        name: parts[1].to_string(),
                    });
                }
            }
        }
    }
    out
}

pub fn sync(repo: &RepoRef) -> Result<PathBuf> {
    let path = mirror_path(repo);
    if path.join("HEAD").exists() {
        fetch(repo, &path)?;
    } else {
        clone_mirror(repo, &path)?;
    }
    Ok(path)
}

/// Network guard: abort a transfer that stalls (GitHub down, captive portal,
/// dropped connection) instead of hanging the request forever. A hard
/// wall-clock kill would also kill legitimately large clones; low-speed
/// detection only trips on transfers that have genuinely stopped moving.
const LOW_SPEED_LIMIT: &str = "1000"; // bytes/s
const LOW_SPEED_TIME: &str = "30"; // seconds

/// git with prompts disabled — a credential prompt on a headless server would
/// block the request forever, which is the usual "GitHub is down" hang.
fn git() -> Command {
    let mut c = Command::new("git");
    c.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_HTTP_LOW_SPEED_LIMIT", LOW_SPEED_LIMIT)
        .env("GIT_HTTP_LOW_SPEED_TIME", LOW_SPEED_TIME);
    c
}

/// Run to completion, capturing stderr so the caller can surface the real
/// reason (the old `.status()` let it reach the server console and reported a
/// generic "git clone failed" for every distinct failure).
fn run(mut cmd: Command, repo: &RepoRef) -> Result<()> {
    let out = cmd.output().context("spawn git")?;
    if out.status.success() {
        return Ok(());
    }
    bail!("{}", explain(&String::from_utf8_lossy(&out.stderr), repo));
}

fn clone_mirror(repo: &RepoRef, dest: &PathBuf) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    // a previous attempt that died mid-clone (or a directory someone dropped
    // here) makes `git clone` refuse the target; clear it so retry works.
    // HEAD is the "this mirror is usable" marker the rest of the crate checks.
    if dest.exists() && !dest.join("HEAD").exists() {
        crate::warn!("removing partial mirror {}", dest.display());
        std::fs::remove_dir_all(dest)
            .with_context(|| format!("clear partial mirror {}", dest.display()))?;
    }
    crate::info!("cloning mirror {}", repo.canonical());
    let mut cmd = git();
    cmd.args(["clone", "--mirror", "-q", &repo.https_url()])
        .arg(dest);
    run(cmd, repo)
}

fn fetch(repo: &RepoRef, mirror: &PathBuf) -> Result<()> {
    let mut cmd = git();
    cmd.args(["fetch", "--all", "--prune", "-q"])
        .current_dir(mirror);
    run(cmd, repo)
}

/// Map git's stderr onto one actionable line. The web UI shows this verbatim,
/// so the common failure modes get a real explanation.
fn explain(stderr: &str, repo: &RepoRef) -> String {
    let s = stderr.to_ascii_lowercase();
    let name = repo.canonical();
    if s.contains("could not resolve host") || s.contains("name or service not known") {
        return format!("{name}: cannot reach github — DNS lookup failed (offline?)");
    }
    if s.contains("failed to connect") || s.contains("connection timed out") || s.contains("operation timed out") {
        return format!("{name}: cannot connect to github — connection timed out");
    }
    if s.contains("low speed") || s.contains("remote end hung up") || s.contains("early eof") {
        return format!("{name}: transfer stalled or dropped by github");
    }
    if s.contains("repository not found") || s.contains("does not appear to be a git repository") {
        return format!("{name}: repository not found (check the name, or it is private)");
    }
    if s.contains("could not read from remote repository") || s.contains("authentication failed") {
        return format!("{name}: access denied — private repo and no credentials configured");
    }
    if s.contains("empty repository") || s.contains("no branches") {
        return format!("{name}: repository has no commits yet");
    }
    match stderr.lines().rev().map(str::trim).find(|l| !l.is_empty()) {
        Some(l) => format!("{name}: {}", l.trim_start_matches("fatal: ")),
        None => format!("{name}: git exited with an error"),
    }
}

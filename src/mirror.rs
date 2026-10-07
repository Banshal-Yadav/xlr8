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
    cache_dir().join("mirrors").join(format!("{}.git", repo.slug()))
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

fn clone_mirror(repo: &RepoRef, dest: &PathBuf) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    crate::info!("cloning mirror {}", repo.canonical());
    let status = Command::new("git")
        .args(["clone", "--mirror", "-q", &repo.https_url()])
        .arg(dest)
        .status()
        .context("spawn git clone")?;
    if !status.success() {
        bail!("git clone failed for {}", repo.canonical());
    }
    Ok(())
}

fn fetch(repo: &RepoRef, mirror: &PathBuf) -> Result<()> {
    let status = Command::new("git")
        .args(["fetch", "--all", "--prune", "-q"])
        .current_dir(mirror)
        .status()
        .context("spawn git fetch")?;
    if !status.success() {
        bail!("git fetch failed for {}", repo.canonical());
    }
    Ok(())
}

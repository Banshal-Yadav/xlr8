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

pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("xlr8")
}

/// Ensure a bare mirror exists locally and fetch updates.
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
    tracing::info!("cloning mirror {}", repo.canonical());
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
    tracing::debug!("fetching {}", repo.canonical());
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

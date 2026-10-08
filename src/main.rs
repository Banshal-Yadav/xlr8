mod cli;
mod diff;
mod github;
mod log;
mod mirror;
mod repo;
mod rewrite;
mod search;
mod server;

use anyhow::{bail, Result};

#[derive(Clone)]
pub struct AppState {
    pub gh: github::GitHubClient,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = cli::parse()?;
    let state = AppState {
        gh: github::GitHubClient::new(),
    };

    let repos_to_sync: Vec<mirror::RepoRef> = match &cli.repo {
        Some(input) => vec![mirror::RepoRef::parse(input)?],
        None if cli.serve => mirror::cached_repos(),
        None => {
            let cached = mirror::cached_repos();
            if cached.is_empty() {
                bail!(
                    "no repositories yet\n\nusage:\n  xlr8 owner/repo          open a repository\n  xlr8 https://github.com/owner/repo\n  xlr8 --serve             serve previously opened repos"
                );
            }
            cached
        }
    };

    let mut opened = Vec::new();
    for r in &repos_to_sync {
        let handle = tokio::task::spawn_blocking({
            let r = r.clone();
            move || mirror::sync(&r)
        });
        match handle.await? {
            Ok(p) => {
                crate::info!("ready: {}", p.display());
                server::note_sync();
                opened.push(p);
            }
            Err(e) => crate::warn!("sync failed for {}: {e}", r.canonical()),
        }
    }

    // prewarm: open the mirror + read HEAD before serving, so the first
    // request skips repo open + pack index load (measured ~50 ms on a laptop)
    for p in &opened {
        let p = p.clone();
        tokio::task::spawn_blocking(move || prewarm(&p)).await?;
    }

    let addr = server::serve(state, cli.port).await?;
    let url = format!("http://{addr}");
    crate::info!("xlr8 serving at {url}");
    if !cli.no_open {
        open_url(&url);
    }

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    }
}

/// Open the mirror up front (repo handle gets cached in `REPOS`) and touch
/// HEAD + tree so first-request work — `gix::open`, pack index load, ref
/// peel — happens at boot instead. Measured ~50 ms off the cold path.
fn prewarm(path: &std::path::Path) {
    let t = std::time::Instant::now();
    let Ok(tsr) = repo::get(path) else {
        return;
    };
    let handle = repo::handle(&tsr);
    let warmed = (|| -> anyhow::Result<()> {
        let head = handle.head_id()?;
        let commit = handle.find_object(head)?.peel_to_commit()?;
        let tree = commit.tree()?;
        // iterate so entry decode errors surface here, not on first request
        for entry in tree.iter() {
            let _ = entry?.mode();
        }
        Ok(())
    })();
    if let Err(e) = warmed {
        crate::debug!("prewarm {}: {e}", path.display());
    } else {
        crate::debug!("prewarm {} in {:?}", path.display(), t.elapsed());
    }
}

fn open_url(url: &str) {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "windows") {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", "", url]);
        c
    } else if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg(url);
        c
    } else {
        let mut c = Command::new("xdg-open");
        c.arg(url);
        c
    };
    let _ = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

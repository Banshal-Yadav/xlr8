use anyhow::{bail, Result};

pub struct Cli {
    pub repo: Option<String>,
    pub port: Option<u16>,
    pub no_open: bool,
    pub serve: bool,
}

const USAGE: &str = "\
xlr8 — GitHub in a box

usage:
  xlr8 <owner/repo | github-url>    open a repository
  xlr8 --serve                      serve previously opened repos

options:
  --port <N>    listen on port N (default: ephemeral)
  --no-open     do not open a browser
  -h, --help    show this help";

pub fn parse() -> Result<Cli> {
    let mut cli = Cli {
        repo: None,
        port: None,
        no_open: false,
        serve: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            "--no-open" => cli.no_open = true,
            "--serve" => cli.serve = true,
            "--port" => {
                let v = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--port needs a value"))?;
                cli.port = Some(parse_port(&v)?);
            }
            _ if a.starts_with("--port=") => {
                cli.port = Some(parse_port(&a["--port=".len()..])?);
            }
            _ if a.starts_with('-') => bail!("unknown flag: {a}\n\n{USAGE}"),
            _ => {
                if cli.repo.is_some() {
                    bail!("unexpected argument: {a}");
                }
                cli.repo = Some(a);
            }
        }
    }
    Ok(cli)
}

fn parse_port(v: &str) -> Result<u16> {
    v.parse()
        .map_err(|_| anyhow::anyhow!("invalid port: {v}"))
}

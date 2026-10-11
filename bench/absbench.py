#!/usr/bin/env python3
"""xlr8 absolute benchmark harness (Windows).

Protocol (matches docs/BENCHMARKS.md):
  git    : `git --git-dir=<mirror> diff --numstat -M <base> <head>`, median of N.
  cold   : kill server, delete every <mirror>.cmp, restart, first HTTP hit per range.
  disk   : kill server (cmp kept), restart, first HTTP hit per range (OID-keyed cache).
  warm   : in-memory repeat x100, median/min/max.

The server is booted with a single tiny repo arg (stress/big) so boot pays one
sync (~1.2s) instead of `--serve`'s six; it still serves every mirror by path.

Usage: python bench/absbench.py [--warm 100] [--reps 5] [--port 8902]
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "xlr8.exe")
MIRRORS = os.path.join(os.environ.get("LOCALAPPDATA", ""), "xlr8", "mirrors")

# tag, mirror slug, owner, repo, base, head
RANGES = [
    ("rg91",        "BurntSushi_ripgrep.git", "BurntSushi", "ripgrep", "14.1.1", "15.1.0"),
    ("rg252",       "BurntSushi_ripgrep.git", "BurntSushi", "ripgrep", "0.1.0", "15.2.0"),
    ("tokio867",    "tokio-rs_tokio.git",     "tokio-rs",  "tokio",   "tokio-1.0.0", "tokio-1.53.2"),
    ("libuv495",    "libuv_libuv.git",        "libuv",     "libuv",   "v1.0.0", "v1.53.0"),
    # fully-qualified: bare `head` is ambiguous (git resolves it to HEAD, not the tag)
    ("stress2000",  "stress_big.git",         "stress",    "big",     "refs/tags/base", "refs/tags/head"),
    ("stressgiant", "stress_big.git",         "stress",    "big",     "refs/tags/base", "refs/tags/head2"),
]


def mirror_path(slug):
    return os.path.join(MIRRORS, slug)


def cmp_dir(slug):
    return mirror_path(slug) + ".cmp"


def start_server(port):
    proc = subprocess.Popen(
        [BIN, "stress/big", "--port", str(port), "--no-open"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
    )
    base = f"http://127.0.0.1:{port}"
    deadline = time.time() + 60
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("server exited during boot")
        try:
            with urllib.request.urlopen(base + "/api/repos", timeout=2) as r:
                if r.status == 200:
                    return proc, base
        except Exception:
            time.sleep(0.1)
    proc.kill()
    raise RuntimeError("server did not become ready")


def stop_server(proc):
    if proc and proc.poll() is None:
        proc.kill()
        proc.wait()


def http_get(url):
    t0 = time.perf_counter()
    with urllib.request.urlopen(url, timeout=180) as r:
        body = r.read()
    return (time.perf_counter() - t0) * 1000.0, body


def git_numstat(slug, base, head):
    t0 = time.perf_counter()
    out = subprocess.run(
        ["git", "--git-dir", mirror_path(slug), "diff", "--numstat", "-M", base, head],
        capture_output=True, text=True,
    )
    ms = (time.perf_counter() - t0) * 1000.0
    files = adds = dels = 0
    for line in out.stdout.splitlines():
        parts = line.split("\t")
        if len(parts) < 3:
            continue
        files += 1
        if parts[0] != "-":
            adds += int(parts[0])
        if parts[1] != "-":
            dels += int(parts[1])
    return ms, files, adds, dels


def totals_from_body(body):
    d = json.loads(body)
    files = len(d.get("files", []))
    adds = sum(f.get("additions", 0) for f in d.get("files", []))
    dels = sum(f.get("deletions", 0) for f in d.get("files", []))
    return files, adds, dels


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--warm", type=int, default=100)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--port", type=int, default=8902)
    args = ap.parse_args()

    log = open(os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt"), "w", encoding="utf-8")

    def emit(s):
        print(s, flush=True)
        log.write(s + "\n")
        log.flush()

    emit(f"# xlr8 absbench  warm={args.warm} reps={args.reps}  {time.strftime('%Y-%m-%d %H:%M:%S')}")
    emit(f"# device: {os.environ.get('PROCESSOR_IDENTIFIER', '?')}")

    # ---- git baselines ----
    git = {}
    for tag, slug, owner, repo, base, head in RANGES:
        samples = [git_numstat(slug, base, head) for _ in range(args.reps)]
        med = statistics.median(s[0] for s in samples)
        _, files, adds, dels = samples[0]
        git[tag] = (med, files, adds, dels)
        emit(f"GIT {tag:14s} med={med:8.1f}ms files={files:5d} +{adds} -{dels}")

    results = {tag: {"cold": [], "disk": [], "warm": []} for tag, *_ in RANGES}

    # ---- cold + disk: one boot per rep, all ranges per boot ----
    for rep in range(args.reps):
        # cold: wipe every cmp, restart, first hit per range
        for _, slug, *_ in RANGES:
            shutil.rmtree(cmp_dir(slug), ignore_errors=True)
        proc, base_url = start_server(args.port)
        try:
            for tag, slug, owner, repo, base, head in RANGES:
                url = f"{base_url}/api/{owner}/{repo}/diff?base={base}&head={head}"
                ms, body = http_get(url)
                results[tag]["cold"].append(ms)
                f, a, d = totals_from_body(body)
                emit(f"  COLD {tag:14s} rep{rep} {ms:8.1f}ms files={f} +{a} -{d}")
        finally:
            stop_server(proc)
        # disk: keep cmp, restart, first hit per range
        proc, base_url = start_server(args.port)
        try:
            for tag, slug, owner, repo, base, head in RANGES:
                url = f"{base_url}/api/{owner}/{repo}/diff?base={base}&head={head}"
                ms, _ = http_get(url)
                results[tag]["disk"].append(ms)
                emit(f"  DISK {tag:14s} rep{rep} {ms:8.1f}ms")
        finally:
            stop_server(proc)

    # ---- warm: one boot, all ranges x100 ----
    proc, base_url = start_server(args.port)
    try:
        for tag, slug, owner, repo, base, head in RANGES:
            url = f"{base_url}/api/{owner}/{repo}/diff?base={base}&head={head}"
            http_get(url)  # prime
            warm = [http_get(url)[0] for _ in range(args.warm)]
            results[tag]["warm"] = warm
            emit(f"  WARM {tag:14s} n={args.warm} med={statistics.median(warm):7.2f}ms "
                 f"min={min(warm):7.2f} max={max(warm):7.2f}")
    finally:
        stop_server(proc)

    # ---- summary ----
    emit("")
    emit("=== SUMMARY (medians) ===")
    emit(f"{'range':14s} {'git':>10s} {'cold':>10s} {'disk':>10s} {'warm':>10s} {'cold/git':>9s} {'warm/git':>9s}")
    for tag, *_ in RANGES:
        g = git[tag][0]
        c = statistics.median(results[tag]["cold"])
        d = statistics.median(results[tag]["disk"])
        w = statistics.median(results[tag]["warm"])
        emit(f"{tag:14s} {g:10.1f} {c:10.1f} {d:10.1f} {w:10.2f} {g/c:9.2f} {g/w:9.1f}")

    log.close()


if __name__ == "__main__":
    main()

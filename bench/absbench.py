#!/usr/bin/env python3
"""xlr8 absolute benchmark harness (Windows).

Protocol (matches docs/BENCHMARKS.md):
  git    : `git --git-dir=<mirror> diff --numstat -M <base> <head>`, median of N.
  cold   : kill server, delete every <mirror>.cmp, restart, first HTTP hit per range.
  disk   : kill server (cmp kept), restart, first HTTP hit per range (OID-keyed cache).
  warm   : in-memory repeat x100 over a persistent connection, median/min/max.

Usage: python bench/absbench.py [--warm 100] [--reps 5] [--threads N] [--port 8902]
"""
import argparse
import json
import os
import shutil
import statistics
import subprocess
import time

from benchlib import (ROOT, KeepAlive, cmp_dir, http_get, mirror_path,
                      start_server, stop_server)

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
    ap.add_argument("--threads", type=int, default=None,
                    help="XLR8_THREADS for the server (default: unset = all cores)")
    args = ap.parse_args()

    log = open(os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt"), "a", encoding="utf-8")

    def emit(s):
        print(s, flush=True)
        log.write(s + "\n")
        log.flush()

    emit("")
    emit(f"# xlr8 absbench  warm={args.warm} reps={args.reps} threads={args.threads or 'default'}  {time.strftime('%Y-%m-%d %H:%M:%S')}")
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
        proc, base_url = start_server(args.port, args.threads)
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
        proc, base_url = start_server(args.port, args.threads)
        try:
            for tag, slug, owner, repo, base, head in RANGES:
                url = f"{base_url}/api/{owner}/{repo}/diff?base={base}&head={head}"
                ms, _ = http_get(url)
                results[tag]["disk"].append(ms)
                emit(f"  DISK {tag:14s} rep{rep} {ms:8.1f}ms")
        finally:
            stop_server(proc)

    # ---- warm: one boot, all ranges x100 over a persistent connection ----
    proc, base_url = start_server(args.port, args.threads)
    ka = KeepAlive(args.port)
    try:
        for tag, slug, owner, repo, base, head in RANGES:
            path = f"/api/{owner}/{repo}/diff?base={base}&head={head}"
            ka.get(path)  # prime
            warm = [ka.get(path)[0] for _ in range(args.warm)]
            results[tag]["warm"] = warm
            emit(f"  WARM {tag:14s} n={args.warm} med={statistics.median(warm):7.2f}ms "
                 f"min={min(warm):7.2f} max={max(warm):7.2f}")
    finally:
        ka.close()
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

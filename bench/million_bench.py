#!/usr/bin/env python3
"""Million-line-diff benchmark: git truth + xlr8 cold/warm/disk.

Usage: python bench/million_bench.py [--warm 100] [--reps 5] [--port 8908]
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
SLUG = "bench_million.git"
MIRROR = os.path.join(MIRRORS, SLUG)
CMP = MIRROR + ".cmp"

RANGES = [("million_add1M", "base", "a1"), ("million_mod500k", "base", "m1")]


def start_server(port):
    proc = subprocess.Popen([BIN, "stress/big", "--port", str(port), "--no-open"],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    base = f"http://127.0.0.1:{port}"
    deadline = time.time() + 60
    while time.time() < deadline:
        if proc.poll() is not None:
            raise RuntimeError("server exited")
        try:
            with urllib.request.urlopen(base + "/api/repos", timeout=2) as r:
                if r.status == 200:
                    return proc, base
        except Exception:
            time.sleep(0.1)
    proc.kill()
    raise RuntimeError("no ready")


def stop(proc):
    if proc and proc.poll() is None:
        proc.kill()
        proc.wait()


def http_get(url):
    t0 = time.perf_counter()
    with urllib.request.urlopen(url, timeout=300) as r:
        body = r.read()
    return (time.perf_counter() - t0) * 1000.0, body


def git_truth(base, head):
    t0 = time.perf_counter()
    out = subprocess.run(["git", "--git-dir", MIRROR, "diff", "--numstat", "-M", base, head],
                         capture_output=True, text=True)
    ms = (time.perf_counter() - t0) * 1000.0
    files = adds = dels = 0
    for line in out.stdout.splitlines():
        p = line.split("\t")
        if len(p) < 3:
            continue
        files += 1
        if p[0] != "-":
            adds += int(p[0])
        if p[1] != "-":
            dels += int(p[1])
    return ms, files, adds, dels


def totals(body):
    d = json.loads(body)
    fs = d.get("files", [])
    return len(fs), sum(f.get("additions", 0) for f in fs), sum(f.get("deletions", 0) for f in fs)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--warm", type=int, default=100)
    ap.add_argument("--reps", type=int, default=5)
    ap.add_argument("--port", type=int, default=8908)
    args = ap.parse_args()

    log = open(os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt"), "a", encoding="utf-8")

    def emit(s):
        print(s, flush=True)
        log.write(s + "\n")
        log.flush()

    emit("")
    emit(f"# million bench  warm={args.warm} reps={args.reps}  {time.strftime('%Y-%m-%d %H:%M:%S')}")

    git = {}
    for tag, base, head in RANGES:
        samples = [git_truth(base, head) for _ in range(args.reps)]
        med = statistics.median(s[0] for s in samples)
        _, f, a, d = samples[0]
        git[tag] = med
        emit(f"GIT {tag:16s} med={med:8.1f}ms files={f} +{a} -{d}")

    res = {tag: {"cold": [], "disk": [], "warm": []} for tag, *_ in RANGES}

    for rep in range(args.reps):
        shutil.rmtree(CMP, ignore_errors=True)
        proc, base_url = start_server(args.port)
        try:
            for tag, base, head in RANGES:
                ms, body = http_get(f"{base_url}/api/bench/million/diff?base={base}&head={head}")
                res[tag]["cold"].append(ms)
                f, a, d = totals(body)
                emit(f"  COLD {tag:16s} rep{rep} {ms:8.1f}ms files={f} +{a} -{d}")
        finally:
            stop(proc)
        proc, base_url = start_server(args.port)
        try:
            for tag, base, head in RANGES:
                ms, _ = http_get(f"{base_url}/api/bench/million/diff?base={base}&head={head}")
                res[tag]["disk"].append(ms)
                emit(f"  DISK {tag:16s} rep{rep} {ms:8.1f}ms")
        finally:
            stop(proc)

    proc, base_url = start_server(args.port)
    try:
        for tag, base, head in RANGES:
            url = f"{base_url}/api/bench/million/diff?base={base}&head={head}"
            http_get(url)
            warm = [http_get(url)[0] for _ in range(args.warm)]
            res[tag]["warm"] = warm
            emit(f"  WARM {tag:16s} n={args.warm} med={statistics.median(warm):7.2f}ms "
                 f"min={min(warm):7.2f} max={max(warm):7.2f}")
    finally:
        stop(proc)

    emit("")
    emit("=== MILLION SUMMARY (medians) ===")
    emit(f"{'range':16s} {'git':>10s} {'cold':>10s} {'disk':>10s} {'warm':>10s} {'cold/git':>9s} {'warm/git':>9s}")
    for tag, *_ in RANGES:
        g = git[tag]
        c = statistics.median(res[tag]["cold"])
        d = statistics.median(res[tag]["disk"])
        w = statistics.median(res[tag]["warm"])
        emit(f"{tag:16s} {g:10.1f} {c:10.1f} {d:10.1f} {w:10.2f} {g/c:9.2f} {g/w:9.1f}")
    log.close()


if __name__ == "__main__":
    main()

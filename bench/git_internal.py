#!/usr/bin/env python3
"""Measure git's own internal time vs wall time per range.

`git diff --numstat -M` wall time includes process creation + git's own init
(~25-30 ms on Windows). GIT_TRACE_PERFORMANCE emits a final
"performance: <secs> s: git command" line measuring the work inside the
process. Subtracting gives the spawn/init overhead that flatters xlr8's
wall-clock comparison in README/BENCHMARKS.

Usage: python bench/git_internal.py [--reps 9]
"""
import argparse
import os
import re
import statistics
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from benchlib import mirror_path  # noqa: E402

RANGES = [
    ("rg91", "BurntSushi_ripgrep.git", "14.1.1", "15.1.0"),
    ("rg252", "BurntSushi_ripgrep.git", "0.1.0", "15.2.0"),
    ("tokio867", "tokio-rs_tokio.git", "tokio-1.0.0", "tokio-1.53.2"),
    ("libuv495", "libuv_libuv.git", "v1.0.0", "v1.53.0"),
    ("stress2000", "stress_big.git", "refs/tags/base", "refs/tags/head"),
    ("stressgiant", "stress_big.git", "refs/tags/base", "refs/tags/head2"),
]

PERF = re.compile(r"performance:\s+([0-9.]+)\s+s:\s*git command")


def measure(slug, base, head, reps):
    env = dict(os.environ)
    env["GIT_TRACE_PERFORMANCE"] = "1"
    wall, internal = [], []
    for _ in range(reps):
        t0 = time.perf_counter()
        p = subprocess.run(
            ["git", "--git-dir", mirror_path(slug), "diff", "--numstat", "-M", base, head],
            capture_output=True, text=True, env=env,
        )
        w = (time.perf_counter() - t0) * 1000.0
        m = PERF.search(p.stderr or "")
        if not m:
            continue
        wall.append(w)
        internal.append(float(m.group(1)) * 1000.0)
    return wall, internal


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--reps", type=int, default=9)
    args = ap.parse_args()

    print(f"{'range':<14} {'wall':>9} {'internal':>9} {'spawn':>8}")
    print("-" * 44)
    rows = []
    for tag, slug, base, head in RANGES:
        wall, internal = measure(slug, base, head, args.reps)
        if not wall:
            print(f"{tag:<14} {'(no trace output)':>28}")
            continue
        w = statistics.median(wall)
        i = statistics.median(internal)
        rows.append((tag, w, i))
        print(f"{tag:<14} {w:8.1f}m {i:8.1f}m {w - i:7.1f}m")

    print()
    print("spawn overhead = wall - internal (git's own init + process creation)")


if __name__ == "__main__":
    main()
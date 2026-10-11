#!/usr/bin/env python3
"""Generate the synthetic 1M-line-diff repo (Windows port of bench/gen_million.sh).

base : 1250 files x 400 lines
a1   : +2500 new files x 400 lines  => +1,000,000 / -0
m1   : first 200 lines of every base file rewritten => +250,000 / -250,000

Usage: python bench/gen_million.py [out_dir]
"""
import os
import shutil
import subprocess
import sys

DIR = sys.argv[1] if len(sys.argv) > 1 else os.path.join(os.environ.get("TEMP", "."), "million-src")


def run(*args, cwd=DIR):
    subprocess.run(args, cwd=cwd, check=True, stdout=subprocess.DEVNULL)


def write(path, lines):
    with open(path, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(lines) + "\n")


def main():
    if os.path.exists(DIR):
        shutil.rmtree(DIR)
    os.makedirs(os.path.join(DIR, "d"))
    run("git", "init", "-q", "-b", "main")
    run("git", "config", "user.email", "bench@xlr8")
    run("git", "config", "user.name", "bench")

    # base: 1250 files x 400 lines
    for i in range(1, 1251):
        write(os.path.join(DIR, "d", f"f{i}.txt"),
              [f"f{i} l{j} s{(i * j) % 9973}" for j in range(1, 401)])
    run("git", "add", "-A")
    run("git", "commit", "-q", "-m", "base")
    run("git", "tag", "base")

    # a1: +2500 new files x 400 lines
    for i in range(1, 2501):
        write(os.path.join(DIR, "d", f"n{i}.txt"),
              [f"n{i} l{j} s{(i * j) % 9973}" for j in range(1, 401)])
    run("git", "add", "-A")
    run("git", "commit", "-q", "-m", "add1m")
    run("git", "tag", "a1")

    # m1: back on base, rewrite first 200 lines of every base file
    run("git", "checkout", "-q", "base")
    for i in range(1, 1251):
        lines = [f"f{i} l{j} NEW{i + j}" for j in range(1, 201)]
        lines += [f"f{i} l{j} s{(i * j) % 9973}" for j in range(201, 401)]
        write(os.path.join(DIR, "d", f"f{i}.txt"), lines)
    run("git", "add", "-A")
    run("git", "commit", "-q", "-m", "mod1m")
    run("git", "tag", "m1")
    run("git", "checkout", "-q", "main")
    print(f"generated: {DIR} (base, a1=+1M lines, m1=1M changed lines)")


if __name__ == "__main__":
    main()

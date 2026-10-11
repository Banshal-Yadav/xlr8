#!/usr/bin/env bash
# Synthetic 1M-line-diff repo: base + two heads (pure-add 1M lines; mod 1M changed lines).
set -euo pipefail
DIR="${1:-/tmp/opencode/million-src}"
rm -rf "$DIR"; mkdir -p "$DIR"; cd "$DIR"
git init -q -b main
git config user.email bench@xlr8; git config user.name bench

# base: 1250 files x 400 lines
mkdir -p d
for i in $(seq 1 1250); do
  for j in $(seq 1 400); do echo "f$i l$j s$((i*j % 9973))"; done > "d/f$i.txt"
done
git add -A; git commit -q -m base
git tag base

# a1: add 2500 brand-new files x 400 lines = +1,000,000 lines (pure additions)
for i in $(seq 1 2500); do
  for j in $(seq 1 400); do echo "n$i l$j s$((i*j % 9973))"; done > "d/n$i.txt"
done
git add -A; git commit -q -m add1m
git tag a1

# m1 (back on base): rewrite first 200 lines of every base file
# => per file +200/-200; 1250 files => 1,000,000 changed lines
git checkout -q base
for i in $(seq 1 1250); do
  { for j in $(seq 1 200); do echo "f$i l$j NEW$((i+j))"; done
    for j in $(seq 201 400); do echo "f$i l$j s$((i*j % 9973))"; done; } > "d/f$i.txt"
done
git add -A; git commit -q -m mod1m
git tag m1
git checkout -q main
echo "generated: $DIR (base, a1=+1M lines, m1=1M changed lines)"

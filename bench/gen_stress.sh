#!/usr/bin/env bash
# Generate the synthetic stress repo used by benchmark.md.
# Usage: bash gen_stress.sh [output_dir]   (default: ./stress-src)
set -euo pipefail

D="${1:-./stress-src}"
rm -rf "$D"
mkdir -p "$D"
cd "$D"
git init -q -b main
git config user.email bench@test
git config user.name bench

# phase 1: 2000 files x 40 lines
for i in $(seq 0 199); do
  mkdir -p "mod_$i"
  for j in $(seq 0 9); do
    f="mod_$i/file_$j.txt"
    for k in $(seq 1 40); do echo "mod=$i file=$j line=$k: lorem ipsum dolor sit amet"; done > "$f"
  done
done
git add -A
git commit -qm "init 2000 files"
git tag base

# phase 2: 100 commits churning 20 files each
for c in $(seq 1 100); do
  for i in $(seq 0 19); do
    f="mod_$(( (c * 7 + i) % 200 ))/file_$(( (c + i) % 10 )).txt"
    echo "churn commit=$c extra line" >> "$f"
  done
  git add -A
  git commit -qm "churn $c"
done

# phase 3: big bang — touch all 2000 files
for i in $(seq 0 199); do
  for j in $(seq 0 9); do
    echo "BANG commit touched file mod_$i/file_$j" >> "mod_$i/file_$j.txt"
  done
done
git add -A
git commit -qm "big bang touches all 2000 files"
git tag head

# phase 4: giant 20k-line file with 40 changed lines
for k in $(seq 1 20000); do echo "giant line $k abcdefghijklmnopqrstuvwxyz"; done > giant.txt
git add -A && git commit -qm "giant 20k-line file"
for k in $(seq 1 20000); do
  if [ $((k % 500)) -eq 0 ]; then echo "MODIFIED giant line $k"; else echo "giant line $k abcdefghijklmnopqrstuvwxyz"; fi
done > giant.txt
git add -A && git commit -qm "modify 40 lines of giant"
git tag head2

echo "done: $(git rev-list --all --count) commits, $(git ls-tree -r HEAD --name-only | wc -l) files at HEAD"
echo "install mirror: git clone --mirror '$D' ~/.cache/xlr8/mirrors/stress_big.git"

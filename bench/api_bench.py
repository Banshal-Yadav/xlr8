#!/usr/bin/env python3
"""API endpoint benchmark (warm x100) on the tokio mirror.

Usage: python bench/api_bench.py [--warm 100] [--port 8910]
"""
import argparse
import os
import statistics
import subprocess
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "xlr8.exe")

ENDPOINTS = [
    ("/refs (all refs)",            "/api/tokio-rs/tokio/refs"),
    ("/commits?limit=50",           "/api/tokio-rs/tokio/commits?limit=50"),
    ("/commits?limit=500",          "/api/tokio-rs/tokio/commits?limit=500"),
    ("/commits?count=1",            "/api/tokio-rs/tokio/commits?count=1"),
    ("/commits?limit=10&ref=tag",   "/api/tokio-rs/tokio/commits?limit=10&ref=tokio-1.53.2"),
]


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


def http_get(url):
    t0 = time.perf_counter()
    with urllib.request.urlopen(url, timeout=300) as r:
        r.read()
    return (time.perf_counter() - t0) * 1000.0


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--warm", type=int, default=100)
    ap.add_argument("--port", type=int, default=8910)
    args = ap.parse_args()

    log = open(os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt"), "a", encoding="utf-8")

    def emit(s):
        print(s, flush=True)
        log.write(s + "\n")
        log.flush()

    emit("")
    emit(f"# api bench  warm={args.warm}  {time.strftime('%Y-%m-%d %H:%M:%S')}")

    proc, base = start_server(args.port)
    try:
        for name, path in ENDPOINTS:
            http_get(base + path)  # prime (cold count-cache build etc.)
            samples = [http_get(base + path) for _ in range(args.warm)]
            emit(f"  API {name:28s} n={args.warm} med={statistics.median(samples):7.2f}ms "
                 f"min={min(samples):7.2f} max={max(samples):8.2f}")
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait()
    log.close()


if __name__ == "__main__":
    main()

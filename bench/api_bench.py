#!/usr/bin/env python3
"""API endpoint benchmark (warm x100 over a persistent connection) on tokio.

Usage: python bench/api_bench.py [--warm 100] [--threads N] [--port 8910]
"""
import argparse
import os
import statistics
import time

from benchlib import ROOT, KeepAlive, start_server, stop_server

ENDPOINTS = [
    ("/refs (all refs)",            "/api/tokio-rs/tokio/refs"),
    ("/commits?limit=50",           "/api/tokio-rs/tokio/commits?limit=50"),
    ("/commits?limit=500",          "/api/tokio-rs/tokio/commits?limit=500"),
    ("/commits?count=1",            "/api/tokio-rs/tokio/commits?count=1"),
    ("/commits?limit=10&ref=tag",   "/api/tokio-rs/tokio/commits?limit=10&ref=tokio-1.53.2"),
]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--warm", type=int, default=100)
    ap.add_argument("--port", type=int, default=8910)
    ap.add_argument("--threads", type=int, default=None)
    args = ap.parse_args()

    log = open(os.path.join(ROOT, "docs", "BENCHMARKS-raw.txt"), "a", encoding="utf-8")

    def emit(s):
        print(s, flush=True)
        log.write(s + "\n")
        log.flush()

    emit("")
    emit(f"# api bench  warm={args.warm} threads={args.threads or 'default'}  {time.strftime('%Y-%m-%d %H:%M:%S')}")

    proc, base = start_server(args.port, args.threads)
    ka = KeepAlive(args.port)
    try:
        for name, path in ENDPOINTS:
            ka.get(path)  # prime (cold count-cache build etc.)
            samples = [ka.get(path)[0] for _ in range(args.warm)]
            emit(f"  API {name:28s} n={args.warm} med={statistics.median(samples):7.2f}ms "
                 f"min={min(samples):7.2f} max={max(samples):8.2f}")
    finally:
        ka.close()
        stop_server(proc)
    log.close()


if __name__ == "__main__":
    main()

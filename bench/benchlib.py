#!/usr/bin/env python3
"""Shared helpers for the xlr8 benchmark harnesses.

The server is booted with a single tiny repo arg (stress/big) so boot pays one
sync (~1.2s) instead of `--serve`'s six; it still serves every mirror by path.
"""
import http.client
import os
import subprocess
import time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
BIN = os.path.join(ROOT, "target", "release", "xlr8.exe")
MIRRORS = os.path.join(os.environ.get("LOCALAPPDATA", ""), "xlr8", "mirrors")


def mirror_path(slug):
    return os.path.join(MIRRORS, slug)


def cmp_dir(slug):
    return mirror_path(slug) + ".cmp"


def start_server(port, threads=None):
    env = dict(os.environ)
    if threads is not None:
        env["XLR8_THREADS"] = str(threads)
    proc = subprocess.Popen(
        [BIN, "stress/big", "--port", str(port), "--no-open"],
        stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env,
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
    """One-shot GET on a fresh connection (cold/disk first-hit semantics)."""
    t0 = time.perf_counter()
    with urllib.request.urlopen(url, timeout=180) as r:
        body = r.read()
    return (time.perf_counter() - t0) * 1000.0, body


class KeepAlive:
    """Persistent-connection GET (what a browser does). Avoids the ~24 ms
    per-request TCP setup spike that pollutes warm medians on Windows."""

    def __init__(self, port):
        self.port = port
        self.conn = None

    def _connect(self):
        self.conn = http.client.HTTPConnection("127.0.0.1", self.port, timeout=180)

    def get(self, path):
        if self.conn is None:
            self._connect()
        t0 = time.perf_counter()
        try:
            self.conn.request("GET", path)
            r = self.conn.getresponse()
            body = r.read()
        except (http.client.HTTPException, OSError):
            self._connect()
            self.conn.request("GET", path)
            r = self.conn.getresponse()
            body = r.read()
        return (time.perf_counter() - t0) * 1000.0, body

    def close(self):
        if self.conn is not None:
            self.conn.close()
            self.conn = None

#!/usr/bin/env python3
"""Diagnostic-only wrapper/daemon traces; never a replacement for overhead_live.

Consumer: PR #4's unexplained long-tail wrapper samples while the owned worker
refuses storage admission. Run unchanged release binaries on a hosted runner.
This file lives only on the diagnostic branch and is not a shipping change.
Tracing perturbs scheduling. Keep all observations, receipt counts and failures;
do not subtract controls, retry for a passing number, or claim an SLO pass.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import signal
import subprocess
import tempfile
import time
import traceback

PHASES = ("untraced-control", "daemon-traced", "both-traced")
SAMPLES = 60
WARMUPS = 5
SYSCALLS = (
    "connect,accept,accept4,read,write,poll,ppoll,epoll_wait,epoll_pwait,"
    "epoll_ctl,futex,clone,clone3,openat,close,newfstatat,fstat,flock,fsync,"
    "fdatasync,rename,renameat,renameat2,clock_nanosleep,execve,wait4,waitid,"
    "getsockopt,setsockopt,mmap,munmap,mprotect,sched_yield"
)


def dump(path: Path, value: object) -> None:
    path.write_text(json.dumps(value, indent=2, sort_keys=True) + "\n")


def system_observation() -> dict[str, object]:
    paths = ["/proc/loadavg", "/proc/pressure/cpu", "/proc/pressure/io",
             "/proc/pressure/memory", "/proc/self/cgroup", "/proc/meminfo"]
    result: dict[str, object] = {"unix_ns": time.time_ns(), "kernel": platform.release()}
    for name in paths:
        try:
            result[name] = Path(name).read_text()
        except OSError as error:
            result[name] = str(error)
    cgroup = str(result.get("/proc/self/cgroup", ""))
    relative = next((line.split("::", 1)[1] for line in cgroup.splitlines()
                     if line.startswith("0::")), "/")
    base = Path("/sys/fs/cgroup") / relative.lstrip("/")
    for name in ("cpu.max", "cpu.stat", "memory.current", "memory.max", "io.stat"):
        try:
            result[str(base / name)] = (base / name).read_text()
        except OSError as error:
            result[str(base / name)] = str(error)
    return result


def trace(arguments: list[str], path: Path) -> list[str]:
    return ["strace", "-f", "-ttt", "-T", "-s", "256", "-o", str(path),
            "-e", "trace=" + SYSCALLS, "--", *arguments]


def run_phase(phase: str, binaries: Path, evidence: Path, runtime: Path) -> None:
    out = evidence / phase
    out.mkdir()
    owned = runtime / phase
    owned.mkdir()
    fake = owned / "fake-rustc"
    fake.write_bytes(b"#!/bin/sh\nexit 0\n")
    fake.chmod(0o755)
    env = {key: value for key, value in os.environ.items() if not key.startswith("RABS_")}
    env.update({"RABS_SOCKET_PATH": str(owned / "rabsd.sock"),
                "RABS_STATE_DIR": str(owned / "state"),
                "RABS_BOOT_MARKER": str(owned / "boot"),
                "RABS_CONFIG": "/nonexistent-rabs-config",
                "RABS_BREAKER_FILE": str(owned / "breaker")})
    daemon_command = [str(binaries / "rabsd")]
    if phase != "untraced-control":
        daemon_command = trace(daemon_command, out / "daemon.strace")
    dump(out / "before.json", system_observation())
    rows: list[dict[str, int]] = []
    with (out / "daemon.stderr").open("wb") as stderr:
        daemon = subprocess.Popen(daemon_command, env=env, stdout=subprocess.DEVNULL,
                                  stderr=stderr, start_new_session=True)
        try:
            deadline = time.monotonic() + 10
            while not Path(env["RABS_SOCKET_PATH"]).exists():
                if daemon.poll() is not None or time.monotonic() >= deadline:
                    raise RuntimeError("owned diagnostic daemon did not become ready")
                time.sleep(0.01)

            def sample(wrapped: bool, label: str) -> tuple[int, int]:
                command = [str(fake), "--crate-name", "fx"]
                if wrapped:
                    command.insert(0, str(binaries / "rabs-wrap"))
                    if phase == "both-traced":
                        command = trace(command, out / (label + ".strace"))
                before = time.time_ns()
                start = time.monotonic_ns()
                result = subprocess.run(command, env=env, stdin=subprocess.DEVNULL,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=10)
                elapsed = (time.monotonic_ns() - start) // 1000
                if result.stdout:
                    (out / (label + ".stdout")).write_bytes(result.stdout)
                if result.stderr:
                    (out / (label + ".stderr")).write_bytes(result.stderr)
                if result.returncode:
                    raise RuntimeError(f"{label}: exit {result.returncode}")
                return before, elapsed

            for index in range(WARMUPS):
                sample(True, f"warmup-{index}")
            for index in range(SAMPLES):
                if index % 2 == 0:
                    _, direct = sample(False, f"direct-{index}")
                before, elapsed = sample(True, f"wrapper-{index}")
                if index % 2:
                    _, direct = sample(False, f"direct-{index}")
                rows.append({"index": index, "wrapper_start_unix_ns": before,
                             "wrapper_us": elapsed, "direct_us": direct})
        finally:
            dump(out / "samples.json", rows)
            dump(out / "after.json", system_observation())
            if daemon.poll() is None:
                os.killpg(daemon.pid, signal.SIGTERM)
            try:
                daemon.wait(timeout=15)
            except subprocess.TimeoutExpired:
                os.killpg(daemon.pid, signal.SIGKILL)
                daemon.wait(timeout=5)
                raise RuntimeError("diagnostic daemon needed forced cleanup")
    receipts = (owned / "state/shadow-receipts.ndjson").read_bytes()
    (out / "shadow-receipts.ndjson").write_bytes(receipts)
    count = len(receipts.splitlines())
    summary = {"phase": phase, "classification": "diagnostic-only",
               "samples": len(rows), "confirmed_consults": count,
               "daemon_exit": daemon.returncode,
               "p95_us": sorted(row["wrapper_us"] for row in rows)[57]}
    dump(out / "summary.json", summary)
    print(json.dumps(summary), flush=True)
    if count != SAMPLES + WARMUPS or daemon.returncode != 0:
        raise RuntimeError(f"incomplete consults or daemon cleanup: {summary}")
    if phase != "untraced-control":
        slowest = max(rows, key=lambda row: row["wrapper_us"])
        begin = slowest["wrapper_start_unix_ns"] / 1e9
        end = begin + slowest["wrapper_us"] / 1e6
        selected = []
        for line in (out / "daemon.strace").read_text(errors="replace").splitlines():
            match = re.match(r"(?:\[pid\s+)?\d+\]?\s+(\d+\.\d+)\s+", line)
            if match and begin - .005 <= float(match[1]) <= end + .005:
                selected.append(line)
        (out / "slowest-daemon-window.txt").write_text("\n".join(selected) + "\n")
        dump(out / "slowest-sample.json", slowest)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binaries", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if platform.system() != "Linux" or not shutil.which("strace"):
        parser.error("Linux with strace is required")
    binaries = args.binaries.resolve(strict=True)
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=False)
    identity = {}
    for name in ("rabsd", "rabs-wrap"):
        binary = binaries / name
        if not binary.is_file() or not os.access(binary, os.X_OK):
            raise RuntimeError(f"not a release executable: {binary}")
        identity[name] = hashlib.sha256(binary.read_bytes()).hexdigest()
    dump(output / "binaries.json", identity)
    runtime = Path(tempfile.mkdtemp(prefix="rabs-trace-", dir="/tmp"))
    print("Diagnostic runtime retained at " + str(runtime), flush=True)
    failures = []
    for phase in PHASES:
        try:
            run_phase(phase, binaries, output, runtime)
        except Exception as error:
            failures.append({"phase": phase, "error": repr(error)})
            traceback.print_exc()
    dump(output / "diagnostic-failures.json", failures)
    print("All phases retained. No SLO success or speedup is claimed.", flush=True)
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())

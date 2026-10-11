#!/usr/bin/env python3
"""Linux/Unix PTY regressions and first-frame/RSS measurements. A terminal emulator is not a physical font test."""

import argparse
import codecs
import fcntl
import json
import os
import re
import statistics
from pathlib import Path
import select
import signal
import struct
import subprocess
import termios
import time

import pyte


class TerminalProcess:
    def __init__(self, command, rows=30, columns=100, pixels=(1000, 600)):
        self.master, self.slave = os.openpty()
        self.original_attributes = termios.tcgetattr(self.slave)
        self.screen = pyte.Screen(columns, rows)
        self.stream = pyte.Stream(self.screen)
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.raw = bytearray()
        self.set_size(rows, columns, pixels)

        def acquire_terminal():
            os.setsid()
            fcntl.ioctl(self.slave, termios.TIOCSCTTY, 0)

        self.started = time.perf_counter()
        self.process = subprocess.Popen(command, stdin=self.slave, stdout=self.slave, stderr=self.slave,
                                        preexec_fn=acquire_terminal, env={**os.environ, "TERM": "xterm-256color", "TZ": "UTC"})

    def set_size(self, rows, columns, pixels):
        fcntl.ioctl(self.slave, termios.TIOCSWINSZ, struct.pack("HHHH", rows, columns, *pixels))
        self.screen.resize(lines=rows, columns=columns)

    def pump(self, seconds=0.05):
        if select.select([self.master], [], [], seconds)[0]:
            try:
                data = os.read(self.master, 1 << 20)
            except OSError:
                data = b""
            self.raw.extend(data)
            self.stream.feed(self.decoder.decode(data))

    def until(self, predicate, timeout=10):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            self.pump()
            if predicate():
                return
            if self.process.poll() is not None:
                raise AssertionError(f"Process exited {self.process.returncode}: {bytes(self.raw)[-2000:]!r}")
        raise TimeoutError(f"Timed out; screen:\n{self.text()}")

    def text(self):
        return "\n".join(self.screen.display)

    def settle(self):
        """Drain a complete redraw before sending the next resize; paused scenes then produce no diffs."""
        deadline = time.monotonic() + 3.0
        while time.monotonic() < deadline and select.select([self.master], [], [], 0.15)[0]:
            self.pump(0)

    def send(self, text):
        os.write(self.master, text.encode())

    def wait_exit(self, timeout=10):
        end = time.monotonic() + timeout
        while self.process.poll() is None and time.monotonic() < end:
            self.pump()
        if self.process.poll() is None:
            raise TimeoutError("Process did not exit")
        self.pump(0)
        return self.process.returncode

    def assert_restored(self):
        assert termios.tcgetattr(self.slave) == self.original_attributes, "terminal attributes were not restored"
        assert b"\x1b[?1049l" in self.raw and b"\x1b[?25h" in self.raw, "alternate screen/cursor not restored"

    def close(self):
        if self.process.poll() is None:
            self.process.kill()
            self.process.wait(timeout=5)
        os.close(self.master)
        os.close(self.slave)


def check_star_diagnostics(binary):
    terminal = TerminalProcess([str(binary), "-d", "2025-03-01T11:00:00", "-i", "Tokyo", "-s", "0",
                                "--debug-frametimes"])
    try:
        terminal.until(lambda: "Star fallbacks:" in terminal.text())
        assert "0 catalog, 0 frame" in terminal.text()
        assert "Candidate cells:" in terminal.text() and "Candidate stars:" in terminal.text()
        assert "Evaluated stars:" in terminal.text()
        assert b"Catalog: 0 stars use tangential motion" in terminal.raw
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return {"stellar_fallback_diagnostics": "passed"}
    finally:
        terminal.close()


def read_memory(pid):
    fields = Path(f"/proc/{pid}/status").read_text().splitlines()
    return {line.split(":")[0]: int(line.split()[1]) for line in fields if line.startswith(("VmRSS:", "VmHWM:"))}


def check_interaction(binary, metadata):
    command = [str(binary), "-d", "2025-03-01T11:00:00", "-i", "Tokyo", "-s", "0", "-cCu", "--fps", "30"]
    if metadata:
        command.append("-m")
    terminal = TerminalProcess(command)
    try:
        terminal.until(lambda: "Speed:" in terminal.text() if metadata else sum(c != " " for c in terminal.text()) > 100)
        terminal.settle()
        before = terminal.text()
        terminal.send("\x1b[C" * 3)
        terminal.until(lambda: "207.0" in terminal.text() if metadata else terminal.text() != before)
        terminal.settle()
        terminal.send("++")
        if metadata:
            terminal.until(lambda: "115.2" in terminal.text())
        terminal.settle()
        for rows, columns in [(24, 80), (40, 120), (30, 100)]:
            previous_bytes = len(terminal.raw)
            terminal.set_size(rows, columns, (columns * 10, rows * 20))
            os.kill(terminal.process.pid, signal.SIGWINCH)
            terminal.until(lambda: b"\x1b[2J" in terminal.raw[previous_bytes:] and ("Speed:" in terminal.text() if metadata else True))
            terminal.settle()
            assert len(terminal.screen.display) == rows
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return {"metadata": metadata, "repeat_pan_zoom_resize_quit": "passed"}
    finally:
        terminal.close()


def check_probe(probe, mode, pixels=(1000, 600), expected=None):
    terminal = TerminalProcess([str(probe), mode], pixels=pixels)
    try:
        code = terminal.wait_exit()
        if mode == "panic":
            assert code != 0 and b"intentional terminal restoration probe" in terminal.raw
            terminal.assert_restored()
        else:
            assert code == 0 and expected.encode() in terminal.raw
        return {"mode": mode, "pixels": pixels, "result": "passed"}
    finally:
        terminal.close()


def measure_startup(binary, dataset):
    command = [str(binary), "-d", "2025-03-01T11:00:00", "-i", "Tokyo", "-s", "0", "-m", "--fps", "24"]
    if dataset:
        command += ["--dataset", str(dataset)]
    terminal = TerminalProcess(command)
    try:
        terminal.until(lambda: "Speed:" in terminal.text(), timeout=60)
        elapsed = time.perf_counter() - terminal.started
        until = time.monotonic() + 1.0
        while time.monotonic() < until:
            terminal.pump()
        memory = read_memory(terminal.process.pid)
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return {"dataset": str(dataset) if dataset else "embedded", "first_frame_seconds": elapsed,
                "steady_rss_kib": memory["VmRSS"], "peak_rss_kib": memory["VmHWM"],
                "screen": "30x100 cells; 1000x600 pixels", "page_cache": "uncontrolled; no cache dropped"}
    finally:
        terminal.close()


def measure_workloads(binary, dataset):
    """Read real debug-panel timings and bound input latency by the first frame with changed Facing metadata."""
    results = []
    for threshold, fov in [(5, 180), (12, 10), (12, 180)]:
        for refraction in [False, True]:
            for constellations in [False, True]:
                command = [str(binary), "-d", "2025-03-01T11:00:00", "-i", "Tokyo", "-s", "0",
                           "--debug-frametimes", "--fps", "24", "-F", "225", "-T", "30", "-z", str(fov),
                           "-t", str(threshold), "-cbu"]
                if dataset:
                    command += ["--dataset", str(dataset)]
                if refraction:
                    command += ["-R"]
                if constellations:
                    command += ["-C"]
                terminal = TerminalProcess(command, rows=55, columns=160, pixels=(1600, 1100))
                try:
                    terminal.until(lambda: "Present:" in terminal.text(), timeout=90)
                    end = time.monotonic() + 1.5
                    while time.monotonic() < end:
                        terminal.pump()
                    text = terminal.text()
                    times = {name: float(value) for name, value in re.findall(
                        r"(Frame Time|Solar-system simulation|Observer preparation|Star selection|Stellar simulation|Observation|Projection|Draw|Present):\s*([0-9.]+) ms", text)}
                    counts = {label: int(value) for label, value in re.findall(
                        r"(Candidate cells|Candidate stars|Evaluated stars):\s*(\d+)", text)}
                    assert len(counts) == 3, text
                    facing = lambda: re.search(r"Facing:\s*([0-9.]+)°", terminal.text()).group(1)
                    latencies = []
                    for _ in range(10):
                        before = facing()
                        start = time.perf_counter()
                        terminal.send("l")
                        terminal.until(lambda: facing() != before)
                        latencies.append((time.perf_counter() - start) * 1000)
                    memory = read_memory(terminal.process.pid)
                    terminal.send("q")
                    assert terminal.wait_exit() == 0
                    terminal.assert_restored()
                    results.append({"threshold": threshold, "fov": fov, "refraction": refraction,
                                    "constellations": constellations, "debug_ms_ema": times,
                                    "cells": counts["Candidate cells"], "candidates": counts["Candidate stars"],
                                    "evaluated": counts["Evaluated stars"], "key_to_changed_frame_ms": latencies,
                                    "median_input_ms": statistics.median(latencies), "max_input_ms": max(latencies),
                                    "rss_kib": memory, "screen": "55x160 cells; 1600x1100 pixels"})
                finally:
                    terminal.close()
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/astroterm"))
    parser.add_argument("--probe", type=Path, default=Path("target/release/examples/terminal_probe"))
    parser.add_argument("--dataset", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--workloads", action="store_true", help="also measure the phase-4 matrix and input latency")
    args = parser.parse_args()
    results = {"checks": [check_interaction(args.binary.resolve(), metadata) for metadata in [False, True]],
               "stellar_diagnostics": check_star_diagnostics(args.binary.resolve()),
               "probes": [check_probe(args.probe.resolve(), "panic"),
                          check_probe(args.probe.resolve(), "aspect", pixels=(1000, 300), expected="aspect=1.000000"),
                          check_probe(args.probe.resolve(), "aspect", pixels=(0, 0), expected="aspect=2.000000")],
               "startup": [measure_startup(args.binary.resolve(), None)],
               "limits": ["pyte/wcwidth emulation does not validate actual font width for ambiguous glyphs such as ⬤",
                          "PTY repeat sends repeated key sequences; physical keyboard autorepeat/platform delivery not tested",
                          "Linux only; macOS and Windows not tested"]}
    if args.dataset:
        results["startup"].append(measure_startup(args.binary.resolve(), args.dataset.resolve()))
    if args.workloads:
        results["workloads"] = measure_workloads(args.binary.resolve(), args.dataset.resolve() if args.dataset else None)
    if args.output:
        args.output.write_text(json.dumps(results, indent=2) + "\n")
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()

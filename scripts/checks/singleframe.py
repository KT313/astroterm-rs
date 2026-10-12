#!/usr/bin/env python3
"""One-frame exit, the seven-step execution report, protocol presentation and terminal restoration (BSC, t5)."""
import argparse
import re
from pathlib import Path
import fcntl
import os
import select
import struct
import subprocess
import termios
import time


def capture_frame(command, kitty):
    master, slave = os.openpty()
    attributes = termios.tcgetattr(slave)
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 60, 480, 384))

    def acquire_terminal():
        os.setsid()
        fcntl.ioctl(slave, termios.TIOCSCTTY, 0)

    process = subprocess.Popen(command, stdin=slave, stdout=slave, stderr=slave,
                               preexec_fn=acquire_terminal, env={**os.environ, "TERM": "xterm-256color", "TZ": "UTC"})
    raw = bytearray()
    replied = False
    try:
        deadline = time.monotonic() + 20
        while True:
            if select.select([master], [], [], 0.05)[0]:
                raw.extend(os.read(master, 1 << 20))
            elif process.poll() is not None:
                break
            if kitty and not replied and b"i=32,s=1,v=1,a=q,t=d,f=24,o=z;" in raw:
                os.write(master, b"\x1b_Gi=31;OK\x1b\\\x1b_Gi=32;OK\x1b\\\x1b[6;16;8t\x1b[0n")
                replied = True
            if time.monotonic() > deadline:
                raise TimeoutError("single-frame process did not exit")
        assert process.returncode == 0, bytes(raw[-2000:])
        assert termios.tcgetattr(slave) == attributes, "terminal attributes not restored"
        assert b"\x1b[?1049l" in raw and b"\x1b[?25h" in raw, "screen/cursor not restored"
        return bytes(raw)
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)


def check(binary, name, flags):
    raw = capture_frame([str(binary), "--debug-singleframe", "-i", "Tokyo", "-d", "2025-03-01T11:00:00",
                         "-t", "5", "-s", "100", "--fps", "1", *flags], name == "kitty")
    start = raw.index(b"astroterm --debug-singleframe: execution trace")
    assert raw.rindex(b"\x1b[?1049l") < start, "report must follow terminal restoration"
    report = raw[start:].decode().replace("\r\n", "\n")
    assert "\x1b" not in report, "report must be plain text"
    assert "Presented frames: 1." in report
    assert "UTC JD=2460735.958333333" in report, "single frame must use exact requested UTC"
    names = re.findall(r"^\d+ +(.*?): [\d.]+ ms$", report, re.M)
    steps = ["Solar system", "Observation", "Selection", "Regions", "Projection", "Points", "Present"]
    for step in steps:
        assert names.count(step) == 1, step
    assert [name for name in names if name in steps] == steps, "the seven steps run once each, in order"
    for before in ["Dataset loading", "Group tables", "Terminal setup"]:
        assert names.index(before) < names.index("Solar system"), before
    assert "removed placeholders=14; output stars=9096" in report
    assert "group tables=" in report and "constellation stars=692" in report
    assert "regions=" in report and "patch radius=" in report
    assert "groups simulated=" in report and "images drawn=" in report
    assert "sent " in report and "bytes" in report
    assert "direct diagnostics=" in report and "self/unattributed=" in report
    if name == "kitty":
        assert len(re.findall(rb"\x1b_Ga=p,", raw[:start])) == 1, "exactly one Kitty placement"
    elif name == "iterm2":
        assert raw[:start].count(b"\x1b]1337;File=") == 1
    return report

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("--report-dir", type=Path)
    args = parser.parse_args()
    cases = [(protocol, ["--graphics-protocol", protocol, "-C", "-m"]) for protocol in ["halfblocks", "sixel", "iterm2", "kitty"]]
    cases.append(("timings", ["--graphics-protocol", "kitty", "-C", "--debug-frametimes"]))
    for name, flags in cases:
        report = check(args.binary.resolve(), name, flags)
        if args.report_dir:
            args.report_dir.mkdir(parents=True, exist_ok=True)
            (args.report_dir / f"{name}.txt").write_text(report)
        print(f"{name}: one frame, ordered report, terminal restored")


if __name__ == "__main__":
    main()

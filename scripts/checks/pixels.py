#!/usr/bin/env python3
"""PTY protocol transport/lifecycle checks. This does not emulate graphics or certify a real terminal's display."""
import argparse
import base64
import json
import os
import re
from pathlib import Path
import signal
import time
import zlib
from terminal import TerminalProcess


def track_transport(terminal, marker):
    """Search each incoming chunk once; repeatedly scanning multi-megabyte Kitty payloads distorts latency."""
    original_pump = terminal.pump
    terminal.recent = b""
    terminal.seen_image = False
    terminal.seen_clear = False
    terminal.seen_sync_start = False
    terminal.seen_sync_end = False
    terminal.frame_count = 0
    def pump(seconds=0.05):
        previous = len(terminal.raw)
        original_pump(seconds)
        incoming = terminal.recent[-32:] + bytes(terminal.raw[previous:])
        terminal.seen_image |= marker in incoming
        terminal.seen_clear |= b"\x1b[2J" in incoming
        terminal.seen_sync_start |= b"\x1b[?2026h" in incoming
        terminal.seen_sync_end |= b"\x1b[?2026l" in incoming
        prefix_size = min(32, len(terminal.recent))
        terminal.frame_count += sum(match.end() > prefix_size for match in re.finditer(rb"\x1b\[\?2026l", incoming))
        terminal.recent = (terminal.recent + bytes(terminal.raw[previous:]))[-65536:]
        if len(terminal.raw) > 1 << 20:
            del terminal.raw[:-(1 << 20)]
    terminal.pump = pump


def extract_png(raw):
    matches = list(re.finditer(rb"\x1b\]1337;File=[^:\x07]*:([A-Za-z0-9+/=]+)\x07", raw))
    assert matches, "missing complete iTerm2 PNG"
    png = base64.b64decode(matches[-1][1])
    assert png.startswith(b"\x89PNG\r\n\x1a\n")
    return png


def check_protocol(binary, protocol, capture_dir=None):
    terminal = TerminalProcess([str(binary), "--renderer", "pixels", "--graphics-protocol", protocol,
                                "-d", "2025-03-01T11:00:00", "-i", "Tokyo", "-s", "0", "-C", "-m", "--fps", "4"],
                               rows=80, columns=100, pixels=(1000, 1600))
    terminal.stream.feed = lambda _: None  # pyte does not interpret graphics payloads; inspect transport bytes only
    marker = {"kitty": b"\x1b_G", "sixel": b"\x1bP", "iterm2": b"\x1b]1337;", "halfblocks": "▀".encode()}[protocol]
    track_transport(terminal, marker)
    try:
        terminal.until(lambda: terminal.seen_image and terminal.frame_count >= 1, timeout=20)
        first_frame_ms = (time.perf_counter() - terminal.started) * 1000
        first_png = extract_png(terminal.raw) if protocol == "iterm2" else None
        if first_png is not None and capture_dir:
            capture_dir.mkdir(parents=True, exist_ok=True)
            (capture_dir / "iterm2-first-frame.png").write_bytes(first_png)
        if protocol == "halfblocks":
            assert b"Graphics:" in terminal.raw
        else:
            assert b"Graphics:" not in terminal.raw, "graphics metadata must be in the raster"
        previous_frames = terminal.frame_count
        terminal.send("\x1b[C++")
        terminal.until(lambda: terminal.frame_count >= previous_frames + 2, timeout=20)
        if first_png is not None:
            assert extract_png(terminal.raw) != first_png, "pan/zoom must change the composed image"
        for rows, columns, pixels in [(25, 70, (700, 500)), (31, 60, (600, 620)), (45, 100, (1000, 900)), (30, 90, (0, 0))]:
            terminal.seen_clear = terminal.seen_image = False
            previous_frames = terminal.frame_count
            terminal.recent = b""
            terminal.set_size(rows, columns, pixels)
            os.kill(terminal.process.pid, signal.SIGWINCH)
            terminal.until(lambda: terminal.seen_clear and terminal.seen_image
                           and terminal.frame_count >= previous_frames + 2, timeout=20)
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        assert terminal.seen_sync_start and terminal.seen_sync_end
        if protocol == "kitty":
            assert b"a=d,d=I,i=1953849929" in terminal.raw
        return {"first_frame_ms": round(first_frame_ms, 2), "pan_zoom_resize_quit": "passed",
                "synchronized_frame": "passed", "text": "native cells" if protocol == "halfblocks" else "rasterized"}
    finally:
        terminal.close()


def check_auto(binary):
    terminal = TerminalProcess([str(binary), "--renderer", "pixels", "-m", "-s", "0", "--fps", "2"])
    try:
        terminal.until(lambda: b"half-block output" in terminal.raw, timeout=20)
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return "unanswered query falls back to halfblocks"
    finally:
        terminal.close()


def check_detection(binary, response, expected):
    terminal = TerminalProcess([str(binary), "--renderer", "pixels", "-m", "-s", "0", "--fps", "2"])
    terminal.stream.feed = lambda _: None
    try:
        terminal.until(lambda: b"\x1b[5n" in terminal.raw, timeout=5)
        terminal.send(response)
        marker = {"Kitty": b"a=t", "Sixel": b"\x1bP"}[expected]
        terminal.until(lambda: marker in terminal.raw and b"\x1b[?2026l" in terminal.raw, timeout=10)
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.settle()
        terminal.assert_restored()
        return "reported capability selected; next key received"
    finally:
        terminal.close()


def check_kitty_compression(binary, support, forced):
    command = [str(binary), "--renderer", "pixels", "-d", "2025-03-01T11:00:00", "-i", "Tokyo",
               "-s", "0", "-t", "5", "--fps", "10"]
    if forced:
        command += ["--graphics-protocol", "kitty"]
    terminal = TerminalProcess(command, rows=30, columns=80, pixels=(800, 600))
    terminal.stream.feed = lambda _: None
    try:
        terminal.until(lambda: b"\x1b[5n" in terminal.raw, timeout=5)
        assert b"i=32,s=1,v=1,a=q,t=d,f=24,o=z;" in terminal.raw
        reply = "\x1b_Gi=31;OK\x1b\\"
        if support is not None:
            reply += "\x1b_Gi=32;" + ("OK" if support else "ENOTSUP:compressed payloads are not supported") + "\x1b\\"
        terminal.send(reply + "\x1b[6;20;10t\x1b[0n")
        terminal.until(lambda: bytes(terminal.raw).count(b"\x1b[?2026l") >= 2, timeout=15)
        raw = bytes(terminal.raw)
        images, ids, payload = [], [], b""
        uploading = False
        for header, data in re.findall(rb"\x1b_G([^\x1b;]+);([^\x1b]*)\x1b\\", raw):
            if b"a=t," in header:
                uploading = True
                assert b"f=24," in header
                assert (b"o=z," in header) == bool(support)
                ids.append(int(re.search(rb"i=(\d+)", header)[1]))
            if not uploading:
                continue
            assert len(data) <= 4096
            payload += base64.b64decode(data)
            if b"m=0" in header:
                images.append(zlib.decompress(payload) if support else payload)
                payload, uploading = b"", False
        assert len(images) >= 2 and ids[0] != ids[1]
        assert all(len(image) == 800 * 600 * 3 for image in images)
        assert images[0] == images[1], "paused frames must preserve pixels"
        assert "\U0010eeee".encode() not in raw
        for frame in raw.split(b"\x1b[?2026l")[:2]:
            before, after = frame.split(b"\x1b[?2026h")
            assert b"a=t," in before and b"m=0;" in before
            assert b"m=" not in after and b"\x1b[2J" not in after
            assert after.index(b"a=p") < after.index(b"a=d")
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        tail = bytes(terminal.raw).split(b"\x1b[?2026l")[-1]
        for image_id in [1953849929, 1953849930]:
            assert f"a=d,d=I,i={image_id}".encode() in tail
        return {"compressed": bool(support), "rgb_roundtrip_staged_swap_cleanup": "passed", "forced": forced}
    finally:
        terminal.close()


def check_panic(probe):
    terminal = TerminalProcess([str(probe), "pixel-panic"])
    terminal.stream.feed = lambda _: None
    try:
        assert terminal.wait_exit() != 0
        terminal.settle()
        terminal.assert_restored()
        assert b"a=d,d=I,i=1953849929" in terminal.raw
        assert terminal.raw.rfind(b"\x1b[?2026h") < terminal.raw.rfind(b"\x1b[?2026l") < terminal.raw.rfind(b"\x1b[?1049l")
        return "passed"
    finally:
        terminal.close()


def check_startup_fallback(binary):
    terminal = TerminalProcess([str(binary), "--renderer", "pixels", "--graphics-protocol", "sixel", "-s", "0"],
                               pixels=(65535, 65535))
    try:
        terminal.until(lambda: b"Pixel startup failed; using characters" in terminal.raw, timeout=20)
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return "oversized raster falls back to characters with notice"
    finally:
        terminal.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/astroterm"))
    parser.add_argument("--probe", type=Path, default=Path("target/release/examples/terminal_probe"))
    parser.add_argument("--output", type=Path)
    parser.add_argument("--capture-dir", type=Path, help="save the actual composed iTerm2 PNG for visual inspection")
    args = parser.parse_args()
    result = {protocol: check_protocol(args.binary.resolve(), protocol, args.capture_dir)
              for protocol in ["kitty", "sixel", "iterm2", "halfblocks"]}
    result["detection"] = check_auto(args.binary.resolve())
    result["detect_kitty"] = check_detection(args.binary.resolve(), "\x1b_Gi=31;OK\x1b\\\x1b[6;20;10t\x1b[0n", "Kitty")
    result["detect_sixel"] = check_detection(args.binary.resolve(), "\x1b[?1;2;4c\x1b[6;20;10t\x1b[0n", "Sixel")
    for name, support, forced in [("accepted_auto", True, False), ("accepted_forced", True, True),
                                  ("rejected_forced", False, True), ("unanswered_forced", None, True)]:
        result["compression_" + name] = check_kitty_compression(args.binary.resolve(), support, forced)
    result["panic_restore"] = check_panic(args.probe.resolve())
    result["startup_fallback"] = check_startup_fallback(args.binary.resolve())
    print(json.dumps(result, indent=2))
    if args.output:
        args.output.write_text(json.dumps(result, indent=2) + "\n")

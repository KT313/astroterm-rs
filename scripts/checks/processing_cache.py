#!/usr/bin/env python3
"""Check runtime cache reuse, CLI bypass precedence and paused interaction in a Linux PTY."""
import argparse
import json
from pathlib import Path
import re
from terminal import TerminalProcess


def counts(terminal, label):
    prefix = {"Observation": "Obs", "Projection": "Proj", "Raster": "Raster"}[label]
    match = re.search(rf"{prefix} cache:\s+H:(\d+) R:(\d+) B:(\d+)", terminal.text())
    return tuple(map(int, match.groups())) if match else None


def check(binary, config, disabled):
    command = [str(binary), "--cache-config", str(config), "-i", "Tokyo", "-d", "2025-03-01T11:00:00",
               "-s", "0", "-Ccu", "--fps", "12", "--debug-frametimes"]
    if disabled:
        command.append("--disable-cache")
    terminal = TerminalProcess(command, rows=70, columns=110, pixels=(1100, 1400))
    try:
        def ready():
            values = [counts(terminal, label) for label in ("Observation", "Projection", "Raster")]
            return all(v and (v[2] >= 2 if disabled else v[0] >= 2) for v in values)
        terminal.until(ready, timeout=20)
        before = {label: counts(terminal, label) for label in ("Observation", "Projection", "Raster")}
        if disabled:
            assert all(value[0] == 0 for value in before.values()), before
        terminal.send("\x1b[C++")
        terminal.until(lambda: "115.2" in terminal.text())
        terminal.until(lambda: counts(terminal, "Raster")[1] > before["Raster"][1])
        terminal.send("q")
        assert terminal.wait_exit() == 0
        terminal.assert_restored()
        return {"counts_before_pan_zoom": before, "pan_zoom": "passed", "restoration": "passed"}
    finally:
        terminal.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, default=Path("target/release/astroterm"))
    parser.add_argument("--config", type=Path, default=Path("examples/cache.toml"))
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = {"enabled": check(args.binary.resolve(), args.config.resolve(), False),
              "disabled": check(args.binary.resolve(), args.config.resolve(), True)}
    text = json.dumps(result, indent=2) + "\n"
    print(text)
    if args.output:
        args.output.write_text(text)

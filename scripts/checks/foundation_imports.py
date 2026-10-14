#!/usr/bin/env python3
"""Check the structural rules of CLAUDE.md ("Pipeline restructuring rules") over the whole crate, without
reading Rust.

1. The seven step functions (`simulate_solar_system`, `observe`, `select_regions`, `refresh_regions`,
   `project_stars_to_screen`, `draw_points`, `present`) are called only from `src/pipeline.rs`; their defining
   modules and test code are exempt.
2. At most two module levels below `src/` (`src/a/b/c.rs` is fine, `src/a/b/c/d.rs` is not). The loader folders
   in EXEMPT_DEEP still exceed it; they are listed in the output as pending the owner's decision, not failures.
3. No function has a `_with_progress` or `_with_times` twin: no function name carries either suffix.
4. No `super::super::super` paths.
5. time/ has no imports from other application modules, including in its tests.

Test code is not checked for rule 1: files declared with `#[cfg(test)] mod name;`, files named `tests.rs` or
`*_tests.rs`, and everything after the first line that is exactly `#[cfg(test)]`.

Usage: python3 scripts/checks/foundation_imports.py [--list]   (exit code 1 on violations; --list prints the files)
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "src"

STEPS = ["simulate_solar_system", "observe", "select_regions", "refresh_regions", "project_stars_to_screen", "draw_points", "present"]
STEP_HOMES = {"solar_system", "observation", "selection", "regions", "projection", "points", "present"}
LOOP = SRC / "pipeline.rs"
# Keep-list loader folders deeper than two levels (plan section 3: "may be moved, not rewritten"); owner decides.
EXEMPT_DEEP = ["astro/models", "catalog/mast", "catalog/cache", "sky/catalog", "model/catalog/storage", "terminal/transport/graphics"]


def test_files():
    declared = set()
    for path in SRC.rglob("*.rs"):
        for explicit, module in re.findall(r"#\[cfg\(test\)\]\s*(?:#\[path = \"([^\"]+)\"\]\s*)?mod\s+([a-z_]+)\s*;", path.read_text()):
            folder = path.parent if path.name == "mod.rs" else path.parent / path.stem
            candidates = [path.parent / explicit] if explicit else [folder / f"{module}.rs", folder / module / "mod.rs", path.parent / f"{module}.rs"]
            declared.update(c.resolve() for c in candidates if c.exists())
    return declared


def production_text(path):
    lines = path.read_text().split("\n")
    for index, line in enumerate(lines):
        if line.strip() == "#[cfg(test)]":
            lines = lines[:index]
            break
    return "\n".join(lines)


def module_depth(path):
    relative = path.relative_to(SRC)
    parts = list(relative.parts[:-1])
    if relative.name != "mod.rs":
        parts.append(relative.stem)
    return len(parts) - 1                                     # src/a.rs and src/a/mod.rs are level 0 modules


def main():
    excluded = test_files()
    files = sorted(f for f in SRC.rglob("*.rs") if f.resolve() not in excluded and not (f.name == "tests.rs" or f.name.endswith("_tests.rs")))
    if "--list" in sys.argv:
        for f in files: print(f.relative_to(ROOT))
    failures, exemptions = [], []
    call = {step: re.compile(r"(?<![\w.])" + step + r"\(") for step in STEPS}
    for path in files:
        relative = path.relative_to(ROOT)
        text = production_text(path)
        home = path.relative_to(SRC).parts[0].removesuffix(".rs")
        if path != LOOP and home not in STEP_HOMES:
            for step, pattern in call.items():
                if pattern.search(text) and not re.search(r"\bfn " + step + r"\(", text):
                    failures.append(f"{relative}: calls step function `{step}` outside pipeline.rs")
        for name in re.findall(r"\bfn\s+([a-z_0-9]+)", text):
            if name.endswith("_with_progress") or name.endswith("_with_times"):
                failures.append(f"{relative}: twin function `{name}`")
        if "super::super::super" in text:
            failures.append(f"{relative}: super::super::super path")
        depth = module_depth(path)
        if depth > 2:
            folder = str(path.relative_to(SRC).parent).replace("\\", "/")
            if any(folder == e or folder.startswith(e + "/") for e in EXEMPT_DEEP):
                exemptions.append(f"{relative}: module level {depth}")
            else:
                failures.append(f"{relative}: module level {depth} exceeds two below src/")
    for path in (SRC / "time").rglob("*.rs"):
        if re.search(r"\b(?:crate|astroterm)::(?!time\b)", path.read_text()):
            failures.append(f"{path.relative_to(ROOT)}: time/ depends on another application module")
    for item in failures: print(item)
    print(f"{len(files)} files checked, {len(failures)} violations, {len(exemptions)} deep loader files exempt (owner decision): "
          + ", ".join(sorted({str(pathlib.Path(e.split(':')[0]).parent) for e in exemptions})))
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

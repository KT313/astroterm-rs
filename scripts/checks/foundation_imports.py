#!/usr/bin/env python3
"""Fail when a foundation (keep-list) module imports from the parts of the crate that are being rebuilt or dropped.

Pipeline restructuring, chunk 3 (dev/pipeline-restructuring-plan.md section 3 and chunk 3 of the steps file). The
keep list below is the set of paths that must compile and test without the old processing spine. A violation is
any `crate::...` path in a keep-list file that leads into a forbidden module. Re-exports through `crate::model`,
`crate::sky` and `crate::projection` are resolved to the submodule that defines the symbol by reading the
`pub use submodule::{...}` lines of the respective `mod.rs`.

Test code is not checked: files declared with `#[cfg(test)] mod name;`, files named `tests.rs` or `*_tests.rs`, and
everything after the first line that is exactly `#[cfg(test)]` (the repository keeps inline test modules at the end
of a file).

Usage: python3 scripts/checks/foundation_imports.py [--list]   (exit code 1 on violations; --list prints the files)
"""
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[2]
SRC = ROOT / "src"

# The keep list of plan section 3, as paths. A directory means every non-test .rs file below it.
KEEP = [
    "constants.rs", "astro", "cache", "rows", "progress", "timing", "catalog", "sky/catalog",
    "model/catalog", "model/configuration", "cli", "metadata", "controls",
    "terminal/startup", "terminal/input", "terminal/transport/graphics",
    "scene/text/raster.rs", "scene/raster/pixels/stars.rs",
]

# Module prefixes a keep-list file must not reach. `crate::model`, `crate::sky` and `crate::projection` are allowed
# as roots; their symbols are resolved to submodules through the maps below.
FORBIDDEN = [
    "state", "scene", "canvas", "helpers", "projection::caching",
    "sky::observation", "sky::selection", "sky::simulation", "sky::observer", "sky::positions", "sky::illumination",
    "model::celestial", "model::presentation",
]
ROOT_MAPS = {"model": SRC / "model/mod.rs", "sky": SRC / "sky/mod.rs", "projection": SRC / "projection/mod.rs"}

PATH = re.compile(r"crate::([A-Za-z_][\w:]*?)(?:::\{([^{}]*(?:\{[^{}]*\}[^{}]*)*)\})?(?=[^\w:{]|$)", re.S)
REEXPORT = re.compile(r"pub(?:\(crate\))?\s+use\s+([a-z_]+)::\{([^}]*)\}|pub(?:\(crate\))?\s+use\s+([a-z_]+)::([A-Za-z_]\w*)", re.S)


def symbol_map(mod_rs):
    """symbol -> submodule for every `pub use submodule::...` line of one mod.rs."""
    mapping = {}
    for group_module, group, single_module, single in REEXPORT.findall(mod_rs.read_text()):
        if group_module:
            for item in group.split(","):
                name = item.strip().split(" as ")[-1].strip()
                if name: mapping[name] = group_module
        elif single_module:
            mapping[single] = single_module
    return mapping


def test_files():
    """Files declared as test modules anywhere under src."""
    declared = set()
    for path in SRC.rglob("*.rs"):
        text = path.read_text()
        for name in re.findall(r"#\[cfg\(test\)\]\s*(?:#\[path = \"([^\"]+)\"\]\s*)?mod\s+([a-z_]+)\s*;", text):
            explicit, module = name
            folder = path.parent if path.name == "mod.rs" else path.parent / path.stem
            candidates = [path.parent / explicit] if explicit else [folder / f"{module}.rs", folder / module / "mod.rs", path.parent / f"{module}.rs"]
            declared.update(c.resolve() for c in candidates if c.exists())
    return declared


VISIBILITY = re.compile(r"pub\(in crate(?:::[A-Za-z_]\w*)*\)")


def production_text(path):
    """The file without its trailing inline test module and without `pub(in crate::...)` visibility paths."""
    lines = path.read_text().split("\n")
    for index, line in enumerate(lines):
        if line.strip() == "#[cfg(test)]":
            lines = lines[:index]
            break
    return VISIBILITY.sub("", "\n".join(lines))


def expand(path_text, group):
    """All full paths named by one `crate::a::b::{x, y::{z}}` occurrence."""
    if group is None:
        return [path_text]
    paths = []
    for item in re.split(r",(?![^{]*\})", group):
        item = item.strip()
        if not item: continue
        nested = re.match(r"([A-Za-z_]\w*)::\{([^}]*)\}", item)
        if nested:
            paths.extend(f"{path_text}::{nested.group(1)}::{inner.strip().split(' as ')[0]}" for inner in nested.group(2).split(",") if inner.strip())
        else:
            paths.append(f"{path_text}::{item.split(' as ')[0].strip()}")
    return paths


def violations(path, maps, keep_files):
    found = []
    for match in PATH.finditer(production_text(path)):
        for full in expand(match.group(1), match.group(2)):
            parts = full.split("::")
            root = parts[0]
            resolved = full
            if root in maps and len(parts) >= 2 and parts[1] in maps[root]:
                resolved = "::".join([root, maps[root][parts[1]]] + parts[1:])
            if any(resolved == f or resolved.startswith(f + "::") for f in FORBIDDEN):
                found.append(f"{full} -> {resolved}" if resolved != full else full)
    return sorted(set(found))


def main():
    maps = {root: symbol_map(mod_rs) for root, mod_rs in ROOT_MAPS.items()}
    excluded = test_files()
    keep_files = []
    for entry in KEEP:
        target = SRC / entry
        files = [target] if target.is_file() else sorted(target.rglob("*.rs"))
        keep_files.extend(f for f in files if f.resolve() not in excluded and not (f.name == "tests.rs" or f.name.endswith("_tests.rs")))
    if "--list" in sys.argv:
        for f in keep_files: print(f.relative_to(ROOT))
    failures = 0
    for path in keep_files:
        for item in violations(path, maps, keep_files):
            print(f"{path.relative_to(ROOT)}: {item}")
            failures += 1
    print(f"{len(keep_files)} keep-list files checked, {failures} forbidden imports")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())

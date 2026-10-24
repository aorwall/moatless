#!/usr/bin/env python3
"""Print `true` when the `sandbox` or `moat` binary can differ between two commits.

The inputs are the workspace crates the two binaries depend on, the files
those crates pull in from outside their own directory, the root manifest
entries they use and the locked packages they resolve to. Fails safe: any
error prints `true`. Reasons go to stderr.

Usage: sandbox-rebuild-needed.py <base-rev> [<head-rev>]   (head: HEAD)
"""

import os
import posixpath
import re
import subprocess
import sys

ROOTS = ("moatless-sandbox", "moatless-cli")

# How the binaries are built rather than what they are built from. `ci.yml`
# stays so an edit to the build step republishes what it describes. Also list
# a file a closure crate reads other than through `SOURCE_REF` (a `build.rs`
# read, say): nothing else finds it, so it can change without a rebuild.
ALWAYS = (
    "rust-toolchain.toml",
    ".cargo/config.toml",
    ".cargo/config",
    ".github/workflows/ci.yml",
    ".github/scripts/sandbox-rebuild-needed.py",
)

DEP_TABLES = ("dependencies", "build-dependencies")

# A literal path only: `concat!` arguments are in-crate fixtures, which the
# crate-directory match already covers.
SOURCE_REF = re.compile(r'(?:include_str!|include_bytes!)\(\s*"([^"]+)"|#\[path\s*=\s*"([^"]+)"\]')


class Rebuild(Exception):
    pass


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def read(rev, path):
    try:
        return git("show", f"{rev}:{path}")
    except subprocess.CalledProcessError:
        return None


def parse_toml(text):
    import tomllib

    return tomllib.loads(text) if text is not None else {}


def dep_tables(manifest):
    yield from (manifest.get(t, {}) for t in DEP_TABLES)
    for target in manifest.get("target", {}).values():
        yield from (target.get(t, {}) for t in DEP_TABLES)


def manifest_deps(manifest, workspace_deps):
    """Map each normal and build dependency key to the package name it resolves to."""
    deps = {}
    for table in dep_tables(manifest):
        for key, spec in table.items():
            spec = spec if isinstance(spec, dict) else {}
            if spec.get("workspace"):
                shared = workspace_deps.get(key, {})
                spec = {**(shared if isinstance(shared, dict) else {}), **spec}
            deps[key] = spec.get("package", key)
    return deps


class Workspace:
    def __init__(self, rev):
        self.rev = rev
        self.root = parse_toml(read(rev, "Cargo.toml"))
        ws = self.root.get("workspace", {})
        self.workspace_deps = ws.get("dependencies", {})
        self.members = {}
        for directory in ws.get("members", []):
            manifest = parse_toml(read(rev, f"{directory}/Cargo.toml"))
            name = manifest.get("package", {}).get("name")
            if not name:
                raise Rebuild(f"cannot read the package name of {directory}")
            self.members[name] = (directory, manifest_deps(manifest, self.workspace_deps))

    def closure(self):
        seen, stack = set(), list(ROOTS)
        while stack:
            name = stack.pop()
            if name in seen:
                continue
            if name not in self.members:
                raise Rebuild(f"{name} is not a workspace member")
            seen.add(name)
            stack += [d for d in self.members[name][1].values() if d in self.members]
        return seen

    def outside_inputs(self, crates):
        """Files a closure crate compiles in from outside its own directory."""
        dirs = [self.members[c][0] for c in crates]
        found = set()
        try:
            hits = git("grep", "-lE", r"include_(str|bytes)!|#\[path", self.rev, "--", *(f"{d}/*.rs" for d in dirs))
        except subprocess.CalledProcessError as error:
            if error.returncode == 1:  # no match
                return found
            raise
        # Whole files rather than grep lines: a literal can sit on the line after the macro.
        for path in (line.split(":", 1)[1] for line in hits.splitlines()):
            for match in SOURCE_REF.finditer(read(self.rev, path) or ""):
                ref = posixpath.normpath(posixpath.join(posixpath.dirname(path), match[1] or match[2]))
                if not any(ref.startswith(d + "/") for d in dirs):
                    found.add(ref)
        return found


def root_view(ws, crates):
    """The root manifest minus what cannot reach the closure's build."""
    keys = {k for c in crates for k in ws.members[c][1]}
    view = {k: v for k, v in ws.root.items() if k != "workspace"}
    workspace = {k: v for k, v in ws.root.get("workspace", {}).items() if k not in ("members", "exclude", "metadata")}
    workspace["dependencies"] = {k: v for k, v in ws.workspace_deps.items() if k in keys}
    view["workspace"] = workspace
    return view


def locked_closure(lock, ws, crates):
    """Every locked package the closure resolves to, as (name, version, source, checksum)."""
    packages = lock.get("package", [])
    by_name = {}
    for p in packages:
        by_name.setdefault(p["name"], []).append(p)

    def resolve(entry):
        name, *rest = entry.split(" ")
        candidates = by_name.get(name, [])
        if rest:
            candidates = [p for p in candidates if p["version"] == rest[0]]
        if len(candidates) != 1:
            raise Rebuild(f"cannot resolve lockfile entry '{entry}'")
        return candidates[0]

    seen, stack = {}, [resolve(c) for c in crates]
    while stack:
        p = stack.pop()
        key = (p["name"], p["version"], p.get("source"))
        if key in seen:
            continue
        seen[key] = p.get("checksum")
        deps = p.get("dependencies", [])
        if p["name"] in ws.members and p.get("source") is None:
            # Lockfile edges of a member include its dev-dependencies.
            wanted = set(ws.members[p["name"]][1].values())
            deps = [d for d in deps if d.split(" ")[0] in wanted]
        stack += [resolve(d) for d in deps]
    return seen


def decide(base, head):
    try:
        changed = git("diff", "--name-only", base, head).splitlines()
    except subprocess.CalledProcessError:
        raise Rebuild(f"cannot diff {base}..{head}")

    for path in changed:
        if path in ALWAYS:
            raise Rebuild(f"{path} changed")

    ws = Workspace(head)
    crates = ws.closure()
    dirs = {ws.members[c][0]: c for c in crates}
    print(f"closure: {', '.join(sorted(crates))}", file=sys.stderr)
    outside = ws.outside_inputs(crates)

    for path in changed:
        for directory, crate in dirs.items():
            if path.startswith(directory + "/"):
                raise Rebuild(f"{path} is in {crate}")
        if path in outside:
            raise Rebuild(f"{path} is compiled into a closure crate")

    if "Cargo.toml" in changed:
        if root_view(Workspace(base), crates) != root_view(ws, crates):
            raise Rebuild("the root Cargo.toml changed an entry the closure uses")
        print("Cargo.toml: no change the closure uses", file=sys.stderr)

    if "Cargo.lock" in changed:
        before = locked_closure(parse_toml(read(base, "Cargo.lock")), ws, crates)
        after = locked_closure(parse_toml(read(head, "Cargo.lock")), ws, crates)
        if before != after:
            moved = sorted({k[0] for k in before.keys() ^ after.keys()})
            raise Rebuild(f"Cargo.lock moved a package the closure resolves to: {', '.join(moved)}")
        print("Cargo.lock: no change the closure resolves to", file=sys.stderr)

    return False


def main():
    if len(sys.argv) not in (2, 3):
        print(__doc__, file=sys.stderr)
        return 2
    base = sys.argv[1]
    head = sys.argv[2] if len(sys.argv) == 3 else "HEAD"
    try:
        needed = decide(base, head)
        print("no change reaches the sandbox or moat binaries", file=sys.stderr)
    except Rebuild as reason:
        print(f"rebuild: {reason}", file=sys.stderr)
        needed = True
    except Exception as error:  # noqa: BLE001 - every failure fails safe
        print(f"rebuild: cannot decide ({type(error).__name__}: {error})", file=sys.stderr)
        needed = True
    print("true" if needed else "false")
    return 0


if __name__ == "__main__":
    os.environ.setdefault("GIT_PAGER", "cat")
    sys.exit(main())

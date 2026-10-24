#!/usr/bin/env python3
"""Run sandbox-rebuild-needed.py against a fixture workspace, one commit per case."""

import importlib.util
import os
import subprocess
import sys
import tempfile
import textwrap
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "sandbox-rebuild-needed.py")

ROOT_MANIFEST = """\
[workspace]
members = ["crates/sandbox", "crates/cli", "crates/shared", "crates/server"]

[workspace.dependencies]
shared = {{ path = "crates/shared", package = "moatless-shared" }}
serde = "{serde}"
axum = "{axum}"
"""

CRATES = {
    "crates/sandbox/Cargo.toml": """\
        [package]
        name = "moatless-sandbox"
        [dependencies]
        shared = { workspace = true }
        [dev-dependencies]
        tempfile = "3"
        """,
    "crates/cli/Cargo.toml": """\
        [package]
        name = "moatless-cli"
        [target.'cfg(unix)'.dependencies]
        serde = { workspace = true }
        """,
    "crates/shared/Cargo.toml": """\
        [package]
        name = "moatless-shared"
        """,
    "crates/server/Cargo.toml": """\
        [package]
        name = "moatless-backend"
        [dependencies]
        shared = { workspace = true }
        axum = { workspace = true }
        """,
    "crates/sandbox/src/main.rs": "fn main() {}\n",
    "crates/cli/src/main.rs": 'const C: &str = include_str!(\n    "../../../compose/app.yml"\n);\n',
    "crates/shared/src/lib.rs": "",
    "crates/server/src/main.rs": "fn main() {}\n",
    "compose/app.yml": "a: 1\n",
    "rust-toolchain.toml": '[toolchain]\nchannel = "1.0.0"\n',
}

LOCK = """\
version = 4

[[package]]
name = "moatless-sandbox"
version = "0.1.0"
dependencies = ["moatless-shared", "tempfile"]

[[package]]
name = "moatless-cli"
version = "0.1.0"
dependencies = ["serde"]

[[package]]
name = "moatless-shared"
version = "0.1.0"

[[package]]
name = "moatless-backend"
version = "0.1.0"
dependencies = ["moatless-shared", "axum"]

[[package]]
name = "serde"
version = "{serde}"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "axum"
version = "{axum}"
source = "registry+https://github.com/rust-lang/crates.io-index"
dependencies = ["serde"]

[[package]]
name = "tempfile"
version = "{tempfile}"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""

VERSIONS = {"serde": "1.0.0", "axum": "0.8.0", "tempfile": "3.0.0"}


# Without `tomllib` (Python < 3.11) the script answers `true` to everything.
@unittest.skipUnless(importlib.util.find_spec("tomllib"), "needs Python 3.11+")
class SandboxRebuildNeeded(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.repo = self.dir.name
        self.git("init", "-q")
        for path, text in CRATES.items():
            self.write(path, textwrap.dedent(text))
        self.write_versions(**VERSIONS)
        self.commit()

    def tearDown(self):
        self.dir.cleanup()

    def git(self, *args):
        return subprocess.run(
            ["git", "-c", "user.name=t", "-c", "user.email=t@t", *args],
            cwd=self.repo, check=True, capture_output=True, text=True,
        ).stdout

    def write(self, path, text):
        full = os.path.join(self.repo, path)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w") as f:
            f.write(text)

    def append(self, path, text):
        with open(os.path.join(self.repo, path), "a") as f:
            f.write(text)

    def write_versions(self, **versions):
        self.write("Cargo.toml", ROOT_MANIFEST.format(**versions))
        self.write("Cargo.lock", LOCK.format(**versions))

    def commit(self):
        self.git("add", "-A")
        self.git("commit", "-qm", "c", "--allow-empty")

    def needed(self, base="HEAD~1"):
        out = subprocess.run(
            [sys.executable, SCRIPT, base], cwd=self.repo, check=True, capture_output=True, text=True
        ).stdout.strip()
        return {"true": True, "false": False}[out]

    def test_a_crate_outside_the_closure_needs_no_rebuild(self):
        self.append("crates/server/src/main.rs", "// x\n")
        self.commit()
        self.assertFalse(self.needed())

    def test_a_shared_dependency_needs_a_rebuild(self):
        self.append("crates/shared/src/lib.rs", "pub fn x() {}\n")
        self.commit()
        self.assertTrue(self.needed())

    def test_a_file_included_from_outside_the_crate_needs_a_rebuild(self):
        self.append("compose/app.yml", "b: 2\n")
        self.commit()
        self.assertTrue(self.needed())

    def test_a_lockfile_bump_only_the_backend_uses_needs_no_rebuild(self):
        self.write_versions(**{**VERSIONS, "axum": "0.8.1"})
        self.commit()
        self.assertFalse(self.needed())

    def test_a_lockfile_bump_of_a_dev_dependency_needs_no_rebuild(self):
        self.write_versions(**{**VERSIONS, "tempfile": "3.1.0"})
        self.commit()
        self.assertFalse(self.needed())

    def test_a_lockfile_bump_the_cli_uses_needs_a_rebuild(self):
        self.write_versions(**{**VERSIONS, "serde": "1.0.1"})
        self.commit()
        self.assertTrue(self.needed())

    def test_a_new_member_needs_no_rebuild(self):
        self.write("Cargo.toml", ROOT_MANIFEST.replace('"crates/server"]', '"crates/server", "crates/new"]').format(**VERSIONS))
        self.write("crates/new/Cargo.toml", '[package]\nname = "moatless-new"\n')
        self.commit()
        self.assertFalse(self.needed())

    def test_a_root_profile_change_needs_a_rebuild(self):
        self.append("Cargo.toml", "\n[profile.release]\nlto = true\n")
        self.commit()
        self.assertTrue(self.needed())

    def test_a_toolchain_change_needs_a_rebuild(self):
        self.write("rust-toolchain.toml", '[toolchain]\nchannel = "1.0.1"\n')
        self.commit()
        self.assertTrue(self.needed())

    def test_an_unknown_base_needs_a_rebuild(self):
        self.assertTrue(self.needed(base="0" * 40))


if __name__ == "__main__":
    unittest.main()

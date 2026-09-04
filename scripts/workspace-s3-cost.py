#!/usr/bin/env python3
"""Measure what a per-file S3 workspace mirror would cost against the tar today.

`sandbox/src/init/workspace_sync.rs` uploads one gzipped tar per sync. A mirror
that stores every file under its own key makes each file readable while the
sandbox is down, and charges per object instead of per archive. This script
produces the two numbers that decision needs: the file set a workspace holds
(`measure`), and the monthly bill each layout produces (`cost`).

    scripts/workspace-s3-cost.py measure /opt/moatless/workspace --tar > ws.json
    scripts/workspace-s3-cost.py cost ws.json --uploads-per-day 564

`measure` walks the same file set the tar carries: inside a git repo the
tracked and untracked-non-ignored files from `git ls-files -c -o
--exclude-standard` plus the whole `.git`, and outside one every regular file.
Symlinks and special files are skipped, as they are in `append_tree`.
"""

from __future__ import annotations

import argparse
import gzip
import io
import json
import os
import subprocess
import sys
import tarfile
import time
from dataclasses import dataclass, field
from pathlib import Path

# S3 Standard list prices, eu-north-1, September 2026. Override on the command
# line when the bucket moves or a discount applies.
PRICE_STORAGE_GB_MONTH = 0.023
PRICE_PUT_PER_1K = 0.005
PRICE_GET_PER_1K = 0.0004
# Every PUT is billed at least this many bytes of storage for a month. S3
# Standard has no minimum object size, so this is 0; S3 Standard-IA bills 128 KB
# per object and would change the answer for a small-file mirror.
MIN_BILLED_OBJECT_BYTES = 0

SIZE_BUCKETS = [1024, 4096, 16384, 65536, 262144, 1048576, 16777216]

# Churn windows in seconds: one sync interval, one hour, one day.
CHURN_WINDOWS = [300, 3600, 86400]


@dataclass
class FileSet:
    """Files one workspace would mirror, split by what produces them."""

    files: int = 0
    bytes: int = 0
    tracked_files: int = 0
    tracked_bytes: int = 0
    git_files: int = 0
    git_bytes: int = 0
    loose_files: int = 0
    loose_bytes: int = 0
    size_histogram: dict[str, int] = field(default_factory=dict)
    churn: dict[str, dict[str, int]] = field(default_factory=dict)
    repos: list[str] = field(default_factory=list)

    def add(self, path: Path, size: int, kind: str, mtime: float, now: float) -> None:
        self.files += 1
        self.bytes += size
        if kind == "tracked":
            self.tracked_files += 1
            self.tracked_bytes += size
        elif kind == "git":
            self.git_files += 1
            self.git_bytes += size
        else:
            self.loose_files += 1
            self.loose_bytes += size

        label = size_bucket_label(size)
        self.size_histogram[label] = self.size_histogram.get(label, 0) + 1

        age = now - mtime
        for window in CHURN_WINDOWS:
            key = f"{window}s"
            entry = self.churn.setdefault(key, {"files": 0, "bytes": 0})
            if 0 <= age <= window:
                entry["files"] += 1
                entry["bytes"] += size


def size_bucket_label(size: int) -> str:
    previous = 0
    for edge in SIZE_BUCKETS:
        if size < edge:
            return f"{human(previous)}-{human(edge)}"
        previous = edge
    return f">{human(SIZE_BUCKETS[-1])}"


def human(n: float) -> str:
    for unit in ["B", "KB", "MB", "GB", "TB"]:
        if abs(n) < 1024 or unit == "TB":
            return f"{n:.0f}{unit}" if unit == "B" else f"{n:.1f}{unit}"
        n /= 1024
    return f"{n}"


def git_ls_files(repo: Path) -> list[str] | None:
    """The working-tree file set `append_repo` archives, or None when git fails."""
    try:
        out = subprocess.run(
            ["git", "-C", str(repo), "ls-files", "-c", "-o", "--exclude-standard", "-z"],
            capture_output=True,
            timeout=120,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if out.returncode != 0:
        return None
    return [p for p in out.stdout.decode("utf-8", "replace").split("\0") if p]


def walk_dir(fileset: FileSet, root: Path, kind: str, now: float) -> None:
    """Add every regular file under `root`, skipping symlinks and specials."""
    for dirpath, dirnames, filenames in os.walk(root, followlinks=False):
        dirnames[:] = [d for d in dirnames if not os.path.islink(os.path.join(dirpath, d))]
        for name in filenames:
            path = Path(dirpath) / name
            try:
                st = path.lstat()
            except OSError:
                continue
            if not os.path.isfile(path) or os.path.islink(path):
                continue
            fileset.add(path, st.st_size, kind, st.st_mtime, now)


def measure_tree(fileset: FileSet, root: Path, directory: Path, now: float) -> None:
    """Mirror of `append_tree`: a repo root goes to git, every other dir is walked."""
    if (directory / ".git").is_dir():
        fileset.repos.append(str(directory.relative_to(root)) or ".")
        for rel in git_ls_files(directory) or []:
            path = directory / rel
            try:
                st = path.lstat()
            except OSError:
                continue
            if os.path.islink(path) or not os.path.isfile(path):
                continue
            fileset.add(path, st.st_size, "tracked", st.st_mtime, now)
        walk_dir(fileset, directory / ".git", "git", now)
        return

    try:
        entries = sorted(directory.iterdir())
    except OSError:
        return
    for path in entries:
        try:
            st = path.lstat()
        except OSError:
            continue
        if os.path.islink(path):
            continue
        if path.is_dir():
            measure_tree(fileset, root, path, now)
        elif path.is_file():
            fileset.add(path, st.st_size, "loose", st.st_mtime, now)


def build_tar_size(root: Path, fileset_paths: list[Path]) -> dict[str, int]:
    """Compressed and uncompressed size of the tar the sandbox uploads today."""
    raw = io.BytesIO()
    gz = gzip.GzipFile(fileobj=raw, mode="wb", compresslevel=6)
    counter = {"uncompressed": 0}

    class Counting(io.RawIOBase):
        def writable(self) -> bool:
            return True

        def write(self, b) -> int:  # type: ignore[no-untyped-def]
            counter["uncompressed"] += len(b)
            gz.write(b)
            return len(b)

    with tarfile.open(fileobj=Counting(), mode="w|") as tar:  # type: ignore[arg-type]
        for path in fileset_paths:
            try:
                tar.add(path, arcname=str(path.relative_to(root)), recursive=False)
            except (OSError, ValueError):
                continue
    gz.close()
    return {"tar_bytes": counter["uncompressed"], "tar_gz_bytes": raw.tell()}


def collect_paths(root: Path, directory: Path, out: list[Path]) -> None:
    if (directory / ".git").is_dir():
        for rel in git_ls_files(directory) or []:
            path = directory / rel
            if path.is_file() and not os.path.islink(path):
                out.append(path)
        for dirpath, dirnames, filenames in os.walk(directory / ".git", followlinks=False):
            dirnames[:] = [d for d in dirnames if not os.path.islink(os.path.join(dirpath, d))]
            for name in filenames:
                path = Path(dirpath) / name
                if path.is_file() and not os.path.islink(path):
                    out.append(path)
        return
    try:
        entries = sorted(directory.iterdir())
    except OSError:
        return
    for path in entries:
        if os.path.islink(path):
            continue
        if path.is_dir():
            collect_paths(root, path, out)
        elif path.is_file():
            out.append(path)


def cmd_measure(args: argparse.Namespace) -> int:
    root = Path(args.workspace).resolve()
    if not root.is_dir():
        print(f"not a directory: {root}", file=sys.stderr)
        return 1

    now = time.time()
    fileset = FileSet()
    started = time.time()
    measure_tree(fileset, root, root, now)
    result = {
        "workspace": str(root),
        "measured_at": int(now),
        "walk_seconds": round(time.time() - started, 2),
        "repos": fileset.repos,
        "files": fileset.files,
        "bytes": fileset.bytes,
        "tracked_files": fileset.tracked_files,
        "tracked_bytes": fileset.tracked_bytes,
        "git_files": fileset.git_files,
        "git_bytes": fileset.git_bytes,
        "loose_files": fileset.loose_files,
        "loose_bytes": fileset.loose_bytes,
        "size_histogram": fileset.size_histogram,
        "churn": fileset.churn,
    }

    if args.tar:
        paths: list[Path] = []
        collect_paths(root, root, paths)
        started = time.time()
        result.update(build_tar_size(root, paths))
        result["tar_seconds"] = round(time.time() - started, 2)
        if result["tar_gz_bytes"]:
            result["compression_ratio"] = round(
                result["bytes"] / result["tar_gz_bytes"], 2
            )

    json.dump(result, sys.stdout, indent=2)
    print()
    return 0


def cmd_cost(args: argparse.Namespace) -> int:
    """Monthly cost of each layout, from one measured workspace and fleet rates."""
    with open(args.measurement) as handle:
        ws = json.load(handle)

    files = ws["files"]
    raw_bytes = ws["bytes"]
    tar_gz_bytes = args.tar_gz_bytes or ws.get("tar_gz_bytes")
    if not tar_gz_bytes:
        print("measurement has no tar_gz_bytes; re-run measure --tar or pass --tar-gz-bytes", file=sys.stderr)
        return 1
    ratio = raw_bytes / tar_gz_bytes
    # What one archive occupies fleet-wide. The sample sets the file count and
    # the compression ratio; `--mean-tar-gz-bytes` sets the size, because the
    # fleet mean comes from `moatless_workspace_sync_bytes_sum` and not from one
    # workspace.
    mean_tar_gz = args.mean_tar_gz_bytes or tar_gz_bytes

    churn_key = f"{args.sync_interval}s"
    churn = ws["churn"].get(churn_key)
    if churn is None:
        print(f"measurement has no churn window {churn_key}", file=sys.stderr)
        return 1
    churn_files = max(churn["files"], args.min_churn_files)

    workspaces = args.workspaces
    uploads_per_day = args.uploads_per_day
    downloads_per_day = args.downloads_per_day
    uploads_per_month = uploads_per_day * 30.0
    downloads_per_month = downloads_per_day * 30.0
    uploads_per_workspace = uploads_per_month / max(workspaces, 1)

    gb = 1024.0**3
    mean_object_bytes = mean_tar_gz * ratio / max(files, 1)
    tar_storage_gb = workspaces * mean_tar_gz / gb
    mirror_storage_gb = (
        workspaces * files * max(mean_object_bytes, MIN_BILLED_OBJECT_BYTES) / gb
    )

    # Today: one PUT per upload, one GET per download.
    tar = {
        "layout": "one gzipped tar per task",
        "objects": workspaces,
        "storage_gb": tar_storage_gb,
        "puts": uploads_per_month,
        "gets": downloads_per_month,
    }
    # Delta mirror: PUT only the files whose mtime moved inside the interval,
    # plus one LIST-equivalent per sync to diff against the bucket.
    # Each new workspace still pays one full upload before its first delta.
    first_upload_puts = args.new_workspaces_per_month * files
    mirror_delta = {
        "layout": "per-file mirror, delta sync",
        "objects": workspaces * files,
        "storage_gb": mirror_storage_gb,
        "puts": uploads_per_month * churn_files
        + uploads_per_month * (files / 1000.0)
        + first_upload_puts,
        "gets": downloads_per_month * files,
    }
    # Naive mirror: every sync re-uploads every file.
    mirror_full = {
        "layout": "per-file mirror, full re-upload",
        "objects": workspaces * files,
        "storage_gb": mirror_storage_gb,
        "puts": uploads_per_month * files,
        "gets": downloads_per_month * files,
    }
    # First upload of a workspace, whatever the steady state: every file is new.
    seed_puts = workspaces * files

    rows = []
    for plan in (tar, mirror_delta, mirror_full):
        storage = plan["storage_gb"] * args.price_storage
        put = plan["puts"] / 1000.0 * args.price_put
        get = plan["gets"] / 1000.0 * args.price_get
        rows.append(
            {
                **plan,
                "storage_usd": storage,
                "put_usd": put,
                "get_usd": get,
                "total_usd": storage + put + get,
            }
        )

    print(f"workspace sample: {ws['workspace']}")
    print(
        f"  {files:,} files, {human(raw_bytes)} raw, {human(tar_gz_bytes)} gzipped"
        f" ({ratio:.1f}x)"
    )
    print(
        f"  fleet archive size {human(mean_tar_gz)} gzipped →"
        f" {human(mean_tar_gz * ratio)} as files, {human(mean_object_bytes)} per object"
    )
    print(
        f"  churn per {args.sync_interval}s: {churn['files']:,} files,"
        f" {human(churn['bytes'])} (billed as {churn_files:,} PUTs)"
    )
    print(
        f"fleet: {workspaces:,} live workspaces, {uploads_per_day:,.0f} uploads/day,"
        f" {downloads_per_day:,.0f} downloads/day"
        f" ({uploads_per_workspace:.0f} uploads per workspace per month)"
    )
    print()
    header = f"{'layout':<34}{'objects':>12}{'storage':>10}{'stor $':>9}{'PUT $':>9}{'GET $':>9}{'total $':>10}"
    print(header)
    print("-" * len(header))
    for row in rows:
        print(
            f"{row['layout']:<34}{row['objects']:>12,}{human(row['storage_gb'] * gb):>10}"
            f"{row['storage_usd']:>9.2f}{row['put_usd']:>9.2f}{row['get_usd']:>9.2f}"
            f"{row['total_usd']:>10.2f}"
        )
    print()
    print(
        f"one-time seed of every workspace: {seed_puts:,} PUTs ="
        f" ${seed_puts / 1000.0 * args.price_put:.2f}"
    )
    print(
        f"prices: storage ${args.price_storage}/GB-month, PUT ${args.price_put}/1k,"
        f" GET ${args.price_get}/1k"
    )
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    measure = sub.add_parser("measure", help="walk a workspace and emit its file set as JSON")
    measure.add_argument("workspace", help="workspace dir (WORKSPACE_DIR inside a sandbox)")
    measure.add_argument("--tar", action="store_true", help="also build the gzipped tar and report its size")
    measure.set_defaults(func=cmd_measure)

    cost = sub.add_parser("cost", help="price each layout from a measurement plus fleet rates")
    cost.add_argument("measurement", help="JSON emitted by `measure`")
    cost.add_argument("--workspaces", type=int, default=100, help="live workspaces holding an archive")
    cost.add_argument("--uploads-per-day", type=float, default=564.0)
    cost.add_argument("--downloads-per-day", type=float, default=144.0)
    cost.add_argument("--sync-interval", type=int, default=300, help="churn window to bill, in seconds")
    cost.add_argument("--min-churn-files", type=int, default=1)
    cost.add_argument("--new-workspaces-per-month", type=float, default=0.0)
    cost.add_argument("--tar-gz-bytes", type=int, default=0, help="use when the measurement skipped --tar")
    cost.add_argument(
        "--mean-tar-gz-bytes",
        type=int,
        default=0,
        help="fleet mean archive size (moatless_workspace_sync_bytes_sum / _count)",
    )
    cost.add_argument("--price-storage", type=float, default=PRICE_STORAGE_GB_MONTH)
    cost.add_argument("--price-put", type=float, default=PRICE_PUT_PER_1K)
    cost.add_argument("--price-get", type=float, default=PRICE_GET_PER_1K)
    cost.set_defaults(func=cmd_cost)

    args = parser.parse_args()
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())

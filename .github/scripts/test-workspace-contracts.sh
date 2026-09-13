#!/usr/bin/env bash
set -euo pipefail

cargo test -p moatless-workspace-types --no-default-features
cargo test -p moatless-workspace-types --no-default-features --features schemas
cargo test -p moatless-workspace-types --no-default-features --features openapi
cargo test -p moatless-workspace-types --no-default-features --features schemas,openapi
cargo test -p moatless-workspace-ports --no-default-features
cargo test -p moatless-workspace-ports --all-features

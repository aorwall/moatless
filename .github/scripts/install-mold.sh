#!/usr/bin/env bash
# Install the mold linker and make it the system default.
#
# Takes over /usr/bin/ld, so no `.cargo` config, RUSTFLAGS or linker-driver
# flag is needed and local builds are unaffected.

set -euo pipefail

MOLD_VERSION="${MOLD_VERSION:-2.40.4}"

arch="$(uname -m)"
url="https://github.com/rui314/mold/releases/download/v${MOLD_VERSION}/mold-${MOLD_VERSION}-${arch}-linux.tar.gz"

# The Rust CI jobs run as root inside `rust:1.97-bookworm`, where sudo is not
# installed. The e2e jobs run as a normal user on a GitHub-hosted runner.
if [ "$(id -u)" = 0 ]; then
    SUDO=""
else
    SUDO="sudo"
fi

echo "Installing mold ${MOLD_VERSION} (${arch})..."

curl --proto '=https' --tlsv1.2 -fsSL \
    --retry 5 \
    --retry-delay 3 \
    --retry-all-errors \
    --connect-timeout 10 \
    "$url" \
    | $SUDO tar -C /usr/local --strip-components=1 --no-overwrite-dir -xzf -

# Take over the real `ld` that cc invokes. `realpath` resolves the
# distro's ld -> ld.bfd symlink so the replacement survives alternatives.
current_ld="$(realpath /usr/bin/ld)"
if [ "$current_ld" != /usr/local/bin/mold ]; then
    $SUDO ln -sf /usr/local/bin/mold "$current_ld"
fi

echo "mold ${MOLD_VERSION} installed:"
mold --version
ld --version | head -1

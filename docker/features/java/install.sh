#!/bin/bash
# docker/features/java/install.sh — Eclipse Temurin JDK + Maven.
#
# Runs as root inside a recipe build, after
# `COPY docker/features/java/ /tmp/features/java/` (see docker/generate-recipe.sh).
# The caller sets JAVA_VERSION (feature.json's `version` option, via the
# generator's <FEATURE>_<OPTION> convention) before invoking this script —
# there is no fallback default here on purpose: the recipe pins it, this
# script trusts the recipe.
#
# Mirrors the JDK layer proven in the deleted docker/Dockerfile.sandbox-soap:
# Adoptium apt repo, not Ubuntu's own openjdk package (noble's repos don't
# reliably carry a JDK 25 build; Adoptium gives 21 and 25 the same way).

set -euo pipefail

: "${JAVA_VERSION:?JAVA_VERSION must be set (e.g. JAVA_VERSION=21)}"

# The base image (sandbox-base or sandbox) already ran
# apt-https-mirrors.sh --bootstrap-ca-certificates once at build time; rerunning
# it here is a no-op on an already-HTTPS sources list and cheap insurance if a
# future base ever stops doing it itself.
apt-https-mirrors.sh --bootstrap-ca-certificates

apt-get update
apt-get install -y --no-install-recommends \
    wget apt-transport-https gpg maven

install -d -m 0755 /etc/apt/keyrings
wget -qO - https://packages.adoptium.net/artifactory/api/gpg/key/public \
    | gpg --dearmor -o /etc/apt/keyrings/adoptium.gpg
echo "deb [signed-by=/etc/apt/keyrings/adoptium.gpg] https://packages.adoptium.net/artifactory/deb $(. /etc/os-release && echo "$VERSION_CODENAME") main" \
    > /etc/apt/sources.list.d/adoptium.list

apt-get update
apt-get install -y --no-install-recommends "temurin-${JAVA_VERSION}-jdk"
rm -rf /var/lib/apt/lists/*

# Stable path independent of the exact installed package-version string — same
# convention as the old Dockerfile.sandbox-java / Dockerfile.sandbox-full.
ln -sf /usr/lib/jvm/temurin-${JAVA_VERSION}-jdk-* /usr/lib/jvm/default-java

# The real JAVA_HOME/PATH ENV is emitted into the generated Dockerfile from
# this feature's feature.json "env" field (see generate-recipe.sh) — that's
# what the sandbox process itself inherits. This /etc/profile append is only
# a secondary convenience for interactive login shells, same convention
# sandbox-base uses for RUSTUP_HOME/CARGO_HOME/BUN_INSTALL/VP_HOME.
{
  echo 'export JAVA_HOME=/usr/lib/jvm/default-java'
  echo 'export PATH="/usr/lib/jvm/default-java/bin:$PATH"'
} >> /etc/profile

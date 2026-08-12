#!/bin/bash
# docker/features/postgres-client/install.sh — postgresql-client CLI.
#
# Runs as root inside a recipe build, after
# `COPY docker/features/postgres-client/ /tmp/features/postgres-client/` (see
# docker/generate-recipe.sh).

set -euo pipefail

# The base image (sandbox-base or sandbox) already ran
# apt-https-mirrors.sh --bootstrap-ca-certificates once at build time; rerunning
# it here is a no-op on an already-HTTPS sources list and cheap insurance if a
# future base ever stops doing it itself.
apt-https-mirrors.sh --bootstrap-ca-certificates

apt-get update
apt-get install -y --no-install-recommends postgresql-client
rm -rf /var/lib/apt/lists/*

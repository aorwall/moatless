#!/usr/bin/env bash
# Configure sccache against an S3 bucket, or no-op if one isn't configured.
#
# S3 rather than the Actions cache: that budget is full, so a GHA-backed
# sccache would evict the caches it depends on. With SCCACHE_S3_BUCKET unset
# this exits 0 without setting RUSTC_WRAPPER and callers compile unchanged —
# it deliberately does not fall back to a GHA-backed sccache.
#
# Required to activate:
#   vars.SCCACHE_S3_BUCKET   S3 bucket name — soap-platform-ci-sccache
#   vars.SCCACHE_AWS_ROLE    ARN of GHA-SccacheRW-moatless, assumed via OIDC by
#                            the calling job (see ci.yml)
#   vars.SCCACHE_S3_REGION   bucket region (defaults to eu-north-1, where the
#                            platform ECR registry already lives)
#
# Both the bucket and the role are defined in soap-infra,
# terraform/platform/ci-artifacts/sccache.tf.

set -euo pipefail

SCCACHE_VERSION="${SCCACHE_VERSION:-v0.10.0}"

if [ -z "${SCCACHE_S3_BUCKET:-}" ]; then
    echo "SCCACHE_S3_BUCKET not set — skipping sccache (compiling without a shared object cache)."
    exit 0
fi

# The auth step is continue-on-error, so a failed role assumption reaches here
# with the bucket set but nothing to sign requests with. Compiling without a
# cache is the correct degradation; pointing sccache at a bucket it cannot
# reach is not.
if [ -z "${AWS_ACCESS_KEY_ID:-}" ]; then
    echo "::warning::SCCACHE_S3_BUCKET is set but no AWS credentials were exported — skipping sccache."
    exit 0
fi

arch="$(uname -m)"
case "$arch" in
    x86_64)  target="x86_64-unknown-linux-musl" ;;
    aarch64) target="aarch64-unknown-linux-musl" ;;
    *) echo "::warning::Unsupported arch ${arch} for sccache — skipping."; exit 0 ;;
esac

install_dir="${RUNNER_TEMP:-/tmp}/sccache"
mkdir -p "$install_dir"

archive="sccache-${SCCACHE_VERSION}-${target}.tar.gz"
echo "Installing sccache ${SCCACHE_VERSION} (${target})..."
curl --proto '=https' --tlsv1.2 -fsSL \
    --retry 5 --retry-delay 3 --retry-all-errors --connect-timeout 10 \
    "https://github.com/mozilla/sccache/releases/download/${SCCACHE_VERSION}/${archive}" \
    | tar -xz -C "$install_dir" --strip-components=1

chmod +x "${install_dir}/sccache"

# SCCACHE_BASEDIRS rewrites absolute paths in the compilation inputs to be
# relative to the workspace, so an object cached by one job hits for another
# whose checkout landed at a different path (containerised jobs and host jobs
# do not agree on $GITHUB_WORKSPACE).
{
    echo "RUSTC_WRAPPER=${install_dir}/sccache"
    echo "SCCACHE_BUCKET=${SCCACHE_S3_BUCKET}"
    echo "SCCACHE_REGION=${SCCACHE_S3_REGION:-eu-north-1}"
    echo "SCCACHE_S3_KEY_PREFIX=${SCCACHE_S3_KEY_PREFIX:-moatless/}"
    echo "SCCACHE_BASEDIRS=${GITHUB_WORKSPACE:-$PWD}"
    # An unreachable or throttled bucket must degrade to a plain rustc call,
    # never fail the build.
    echo "SCCACHE_IGNORE_SERVER_IO_ERROR=1"
} >> "${GITHUB_ENV:-/dev/stdout}"

echo "sccache configured against s3://${SCCACHE_S3_BUCKET}"
"${install_dir}/sccache" --version

#!/bin/bash
set -e

# Image ENTRYPOINT for docker-compose and `docker run` only. Kubernetes never
# runs it: the pod `command` (sandbox-k8s pod_builder/containers.rs) replaces the
# ENTRYPOINT, so a step added here skips every Kubernetes sandbox. Put setup
# every launch needs in the image itself (an ENV, or a file under /etc such as
# /etc/codeartifact-env.sh) or in the sandbox binary.

# `gh` builds its git credential helper from `GH_PATH` (also exported by the
# sandbox binary, since K8s skips this script); moatless-git-cred
# answers that helper from the broker and hands every other call to `gh`.
if [ -x /usr/local/bin/moatless-git-cred ]; then
    export GH_PATH=/usr/local/bin/moatless-git-cred
fi

# sccache bootstrap — wrap rustc in the shared S3 compilation cache so a fresh
# sandbox reuses objects an earlier sandbox built instead of recompiling the
# dependency graph. Gated on SCCACHE_S3_BUCKET, so unset == no-op.
#
# Sandboxes hit each other's objects because they agree on every absolute path
# (/opt/cargo, /opt/moatless/workspace/<repo>). CI does not — its checkout and
# CARGO_HOME sit elsewhere, and sccache keys on those paths, so nothing is
# shared across the two despite the common bucket and prefix.
#
# sccache cannot read EKS Pod Identity credentials itself: its signer tries
# profile files, then IMDSv2, then gives up — it never looks at
# AWS_CONTAINER_CREDENTIALS_FULL_URI. So the credentials are handed to the
# server process explicitly, and refreshed before they expire.
if [ -n "$SCCACHE_S3_BUCKET" ] && command -v sccache >/dev/null 2>&1; then
    sccache_creds() { aws configure export-credentials --format env-no-export 2>/dev/null; }

    # Restarting is how the server picks up rotated credentials — it keeps the
    # environment it was started with, and the cache itself lives in S3.
    sccache_start() {
        local creds
        creds="$(sccache_creds)" || return 1
        [ -n "$creds" ] || return 1
        sccache --stop-server >/dev/null 2>&1 || true
        env $creds \
            SCCACHE_BUCKET="$SCCACHE_S3_BUCKET" \
            SCCACHE_REGION="${SCCACHE_S3_REGION:-${AWS_REGION:-eu-north-1}}" \
            SCCACHE_S3_KEY_PREFIX="${SCCACHE_S3_KEY_PREFIX:-moatless/}" \
            sccache --start-server >/dev/null 2>&1
    }

    # Retry the first start: at container boot the Pod Identity agent may not
    # be serving credentials yet, and a single failed attempt used to disable
    # the cache for the pod's whole life — the re-arm loop below only ran on
    # success, so one lost race meant compiling uncached until the pod died.
    sccache_started=false
    for _attempt in 1 2 3 4 5; do
        if sccache_start; then
            sccache_started=true
            break
        fi
        sleep 5
    done

    if [ "$sccache_started" = true ]; then
        export RUSTC_WRAPPER=/usr/local/bin/sccache
        # Pod Identity credentials last ~6h; re-arm inside that window. A failed
        # refresh leaves the old server running until its credentials lapse,
        # after which sccache falls through to plain rustc rather than failing.
        (while sleep 18000; do sccache_start || true; done) &
        echo "[sccache] wrapping rustc against s3://${SCCACHE_S3_BUCKET}" >&2
    else
        echo "[sccache] could not start against s3://${SCCACHE_S3_BUCKET} — compiling uncached" >&2
    fi
fi

exec /usr/local/bin/sandbox

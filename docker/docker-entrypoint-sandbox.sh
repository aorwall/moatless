#!/bin/bash
set -e

# Minimal entrypoint — all initialization (credentials, git, servers) is now
# handled by the Rust sandbox binary's init module.

# Ensure basic directories exist (Rust init will create subdirs)
mkdir -p "$HOME"
mkdir -p "${WORKSPACE_DIR:-/workspace}"
mkdir -p "${DATA_DIR:-/opt/moatless/data}"
mkdir -p "${CLAUDE_CONFIG_DIR:-$HOME/.claude}/debug"

# Point debug logs to /tmp (workaround for SDK lazy-init race condition)
export CLAUDE_CODE_DEBUG_LOGS_DIR="/tmp/claude-debug.log"

# Configure git worktree environment if needed
if [ "$USE_GIT_WORKTREES" = "true" ] && [ -n "$GIT_WORKTREE_NAME" ]; then
    export GIT_DIR="/main-repo-git/worktrees/$GIT_WORKTREE_NAME"
    export GIT_WORK_TREE="$WORKSPACE_DIR"
fi

# CodeArtifact bootstrap — point npm / pip / maven at the org CodeArtifact repos
# so in-task package installs resolve over the CodeArtifact VPC endpoints instead
# of NAT (and resolve private @soaplabs / com.soaplabs packages with no GitHub
# token). Gated on CODEARTIFACT_DOMAIN, so unset == no-op (local/dev unaffected).
# Best-effort: a failed token mint warns and continues rather than aborting the
# sandbox. Requires AWS creds (the moatless-sandbox Pod Identity) + AWS CLI v2.
if [ -n "$CODEARTIFACT_DOMAIN" ]; then
    ca_owner="${CODEARTIFACT_DOMAIN_OWNER:-222222222222}"
    ca_region="${AWS_REGION:-${AWS_DEFAULT_REGION:-eu-north-1}}"
    ca_npm_repo="${CODEARTIFACT_NPM_REPO:-npm}"
    ca_pypi_repo="${CODEARTIFACT_PYPI_REPO:-pypi}"
    ca_maven_repo="${CODEARTIFACT_MAVEN_REPO:-maven}"
    ca_host="${CODEARTIFACT_DOMAIN}-${ca_owner}.d.codeartifact.${ca_region}.amazonaws.com"

    if ! command -v aws >/dev/null 2>&1; then
        echo "[codeartifact] aws CLI not found — skipping registry bootstrap" >&2
    else
        ca_token="$(aws codeartifact get-authorization-token \
            --domain "$CODEARTIFACT_DOMAIN" --domain-owner "$ca_owner" --region "$ca_region" \
            --query authorizationToken --output text 2>/dev/null || true)"

        if [ -z "$ca_token" ] || [ "$ca_token" = "None" ]; then
            echo "[codeartifact] could not mint a token (no AWS creds?) — skipping" >&2
        else
            # Export for repos whose own .npmrc / settings.xml reference
            # ${CODEARTIFACT_AUTH_TOKEN} (the documented contract). A project-level
            # .npmrc shadows ~/.npmrc for the same _authToken key, so writing
            # ~/.npmrc alone is not enough — the env var must resolve too. The Rust
            # sandbox binary (exec'd below) inherits this, as do the setup commands
            # it spawns (`sh -c "… bun install"`).
            export CODEARTIFACT_AUTH_TOKEN="$ca_token"

            # npm + bun (read ~/.npmrc) — fallback for repos that ship no .npmrc
            {
                echo "registry=https://${ca_host}/npm/${ca_npm_repo}/"
                echo "@soaplabs:registry=https://${ca_host}/npm/${ca_npm_repo}/"
                echo "//${ca_host}/npm/${ca_npm_repo}/:_authToken=${ca_token}"
                echo "//${ca_host}/npm/${ca_npm_repo}/:always-auth=true"
            } > "$HOME/.npmrc"

            # pip / uv — env-only, inherited by the sandbox and its setup commands
            export PIP_INDEX_URL="https://aws:${ca_token}@${ca_host}/pypi/${ca_pypi_repo}/simple/"
            export UV_DEFAULT_INDEX="$PIP_INDEX_URL"

            # maven (wildcard mirror). Maven resolves settings from the JVM's
            # user.home, which comes from the passwd entry (/root for uid 0),
            # not $HOME (/home/moatless) — write to both.
            pw_home="$(getent passwd "$(id -u)" | cut -d: -f6)"
            for m2_dir in "$HOME/.m2" "${pw_home:-$HOME}/.m2"; do
                mkdir -p "$m2_dir"
                cat > "$m2_dir/settings.xml" <<EOF
<settings>
  <servers>
    <server>
      <id>codeartifact</id>
      <username>aws</username>
      <password>${ca_token}</password>
    </server>
  </servers>
  <mirrors>
    <mirror>
      <id>codeartifact</id>
      <name>CodeArtifact wildcard mirror</name>
      <url>https://${ca_host}/maven/${ca_maven_repo}/</url>
      <mirrorOf>*</mirrorOf>
    </mirror>
  </mirrors>
</settings>
EOF
            done
            echo "[codeartifact] configured npm/pip/maven for ${ca_host}" >&2
        fi
    fi
fi

# sccache bootstrap — wrap rustc in the shared S3 compilation cache so a fresh
# sandbox reuses objects an earlier sandbox built instead of recompiling the
# dependency graph. Gated on SCCACHE_S3_BUCKET, so unset == no-op (local/docker
# unaffected).
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

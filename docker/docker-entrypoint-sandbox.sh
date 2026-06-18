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

# CodeArtifact bootstrap — point npm / maven at the org CodeArtifact repos so
# in-task package installs resolve over the CodeArtifact VPC endpoints instead
# of NAT (and resolve private @soaplabs / com.soaplabs packages with no GitHub
# token). Gated on CODEARTIFACT_DOMAIN, so unset == no-op (local/dev unaffected).
# Best-effort: a failed token mint warns and continues rather than aborting the
# sandbox. Requires AWS creds (the moatless-sandbox Pod Identity) + AWS CLI v2.
if [ -n "$CODEARTIFACT_DOMAIN" ]; then
    ca_owner="${CODEARTIFACT_DOMAIN_OWNER:-222222222222}"
    ca_region="${AWS_REGION:-${AWS_DEFAULT_REGION:-eu-north-1}}"
    ca_npm_repo="${CODEARTIFACT_NPM_REPO:-npm}"
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
            # npm + bun (read ~/.npmrc)
            {
                echo "registry=https://${ca_host}/npm/${ca_npm_repo}/"
                echo "@soaplabs:registry=https://${ca_host}/npm/${ca_npm_repo}/"
                echo "//${ca_host}/npm/${ca_npm_repo}/:_authToken=${ca_token}"
                echo "//${ca_host}/npm/${ca_npm_repo}/:always-auth=true"
            } > "$HOME/.npmrc"

            # maven (wildcard mirror)
            mkdir -p "$HOME/.m2"
            cat > "$HOME/.m2/settings.xml" <<EOF
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
            echo "[codeartifact] configured npm/maven for ${ca_host}" >&2
        fi
    fi
fi

exec /usr/local/bin/sandbox

# /etc/codeartifact-env.sh: sourced by every bash, non-interactive ones through
# BASH_ENV and interactive ones through /etc/bash.bashrc (Dockerfile.sandbox-base).
# The image wires it there because Kubernetes never runs the ENTRYPOINT.
#
# Exports CODEARTIFACT_AUTH_TOKEN for repos whose .npmrc reads it, and keeps
# ~/.npmrc (@soaplabs scope) and pip.conf on the current token. The token cache
# is shared with /etc/mavenrc. Sourced: never exit, never write to stdout.

_codeartifact_env() {
  [ -n "${CODEARTIFACT_DOMAIN:-}" ] || return 0
  local cache="${XDG_CACHE_HOME:-$HOME/.cache}/codeartifact/token"
  local now="${EPOCHSECONDS:-0}" token="" exp=0 written=""
  # The fast path reads three small files with builtins: no fork per shell.
  [ -s "$cache" ] && read -r token <"$cache"
  [ -s "$cache.exp" ] && read -r exp <"$cache.exp"
  [[ "$exp" =~ ^[0-9]+$ ]] || exp=0
  if [ -z "$token" ] || ((now >= exp - 600)); then
    _codeartifact_mint "$cache" "$now" "$exp"
    token="" exp=0
    [ -s "$cache" ] && read -r token <"$cache"
    [ -s "$cache.exp" ] && read -r exp <"$cache.exp"
    [[ "$exp" =~ ^[0-9]+$ ]] || exp=0
  fi
  [ -n "$token" ] && ((now < exp)) || return 0
  export CODEARTIFACT_AUTH_TOKEN="$token"
  # /etc/mavenrc mints into the same cache without touching npm or pip config.
  [ -s "$cache.written" ] && read -r written <"$cache.written"
  [ "$written" = "$exp" ] || _codeartifact_write "$cache" "$token" "$exp"
}

_codeartifact_host() {
  printf '%s-%s.d.codeartifact.%s.amazonaws.com' "$CODEARTIFACT_DOMAIN" \
    "${CODEARTIFACT_DOMAIN_OWNER:-222222222222}" "${AWS_REGION:-${AWS_DEFAULT_REGION:-eu-north-1}}"
}

# Writes `$2` to `$1` at mode 0600 through a rename, so a concurrent reader
# sees the old file or the new one.
_codeartifact_put() {
  mkdir -p "${1%/*}" && (umask 077 && printf '%s' "$2" >"$1.tmp.$$") && mv -f "$1.tmp.$$" "$1"
}

# No default `registry=` line and no uv config, deliberately: bun.lock and
# uv.lock record the registry URL, and a CodeArtifact URL there fails outside
# the sandbox.
_codeartifact_write() {
  local token="$2" host npm
  host="$(_codeartifact_host)"
  npm="//$host/npm/${CODEARTIFACT_NPM_REPO:-npm}/"
  _codeartifact_put "$HOME/.npmrc" "@soaplabs:registry=https:$npm
$npm:_authToken=$token
$npm:always-auth=true
" &&
    _codeartifact_put "${XDG_CONFIG_HOME:-$HOME/.config}/pip/pip.conf" "[global]
index-url = https://aws:$token@$host/pypi/${CODEARTIFACT_PYPI_REPO:-pypi}/simple/
" &&
    _codeartifact_put "$1.written" "$3"
}

# Mints under a lock, at most once per five minutes after a failure. A failed
# renewal keeps a still-valid token; with none, it writes the failure note.
_codeartifact_mint() {
  local cache="$1" now="$2" held_until="$3"
  command -v aws >/dev/null 2>&1 || return 0
  local failed=0
  [ -s "$cache.failed" ] && read -r failed <"$cache.failed"
  [[ "$failed" =~ ^[0-9]+$ ]] || failed=0
  ((now < failed + 300)) && return 0
  mkdir -p "${cache%/*}" 2>/dev/null || return 0
  (
    flock -n 9 || exit 0
    out="$(timeout 30 aws codeartifact get-authorization-token \
      --domain "$CODEARTIFACT_DOMAIN" --domain-owner "${CODEARTIFACT_DOMAIN_OWNER:-222222222222}" \
      --region "${AWS_REGION:-${AWS_DEFAULT_REGION:-eu-north-1}}" \
      --query '[authorizationToken,expiration]' --output text 2>"$cache.err")"
    token="${out%%[[:space:]]*}"
    if [ -n "$out" ] && [[ "$token" =~ ^[A-Za-z0-9._=-]+$ ]] && [ "$token" != None ]; then
      exp="$(date -d "${out#*[[:space:]]}" +%s 2>/dev/null)" || exp=$((now + 43200))
      _codeartifact_put "$cache" "$token
" && _codeartifact_put "$cache.exp" "$exp
"
      rm -f "$cache.failed" "$cache.err"
      exit 0
    fi
    _codeartifact_put "$cache.failed" "$now
"
    echo "[codeartifact] token mint for domain $CODEARTIFACT_DOMAIN failed:" >&2
    sed '/^[[:space:]]*$/d; s/^/[codeartifact]   /' "$cache.err" >&2
    if ((now >= held_until)); then
      identity="$(timeout 10 aws sts get-caller-identity --query Arn --output text 2>/dev/null)"
      _codeartifact_put "$HOME/.npmrc" "# CodeArtifact token mint FAILED, so there is no auth token.
# get-authorization-token on domain $CODEARTIFACT_DOMAIN said:
$(sed '/^[[:space:]]*$/d; s/^/#   /' "$cache.err")
#   identity: ${identity:-none, no AWS credentials reached this sandbox}
# No private @soaplabs package resolves until that call succeeds. The usual
# cause is a missing codeartifact:GetAuthorizationToken grant on that
# identity, but read the error above rather than assuming it. Do not reach
# for a GitHub Packages token: nothing here publishes to one. The next bash
# started five minutes from now retries and rewrites this file.
"
      rm -f "$cache.written"
    fi
    rm -f "$cache.err"
  ) 9>"$cache.lock"
}

_codeartifact_env

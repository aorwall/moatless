# /etc/sccache-env.sh: sourced by every bash after /etc/codeartifact-env.sh,
# through /etc/sandbox-env.sh (Dockerfile.sandbox-base). The image wires it
# there because Kubernetes never runs the ENTRYPOINT.
#
# With SCCACHE_S3_BUCKET set, wraps rustc in the shared S3 compilation cache.
# Sandboxes agree on every absolute path sccache keys on (/opt/cargo,
# /opt/moatless/workspace/<repo>), so they hit each other's objects; CI's
# checkout and CARGO_HOME differ, so they never hit CI's. Sourced: never exit,
# never write to stdout.

_sccache_env() {
  [ -n "${SCCACHE_S3_BUCKET:-}" ] || return 0
  command -v sccache >/dev/null 2>&1 || return 0
  # Exported to every shell, not only the server's start: a server that dies
  # is restarted by the next compile with the compile's environment. sccache
  # reads EKS Pod Identity credentials itself, from 0.11 on.
  export SCCACHE_BUCKET="$SCCACHE_S3_BUCKET"
  export SCCACHE_REGION="${SCCACHE_S3_REGION:-${AWS_REGION:-eu-north-1}}"
  export SCCACHE_S3_KEY_PREFIX="${SCCACHE_S3_KEY_PREFIX:-moatless/}"
  export SCCACHE_IDLE_TIMEOUT="${SCCACHE_IDLE_TIMEOUT:-0}"
  # Under /tmp so a restarted pod probes the bucket again rather than trusts
  # the last pod's answer.
  local state="${TMPDIR:-/tmp}/sccache-env-$UID" status=""
  [ -s "$state/status" ] && read -r status <"$state/status"
  if [ "$status" != ok ]; then
    _sccache_backing_off "$status" && return 0
    _sccache_start "$state"
    status=""
    [ -s "$state/status" ] && read -r status <"$state/status"
  fi
  # Only after a start succeeded: a wrapper whose server cannot reach the
  # bucket fails every compile, and SCCACHE_IGNORE_SERVER_IO_ERROR does not
  # cover that.
  [ "$status" = ok ] && export RUSTC_WRAPPER=sccache
  return 0
}

# True while status `$1` records a failure under five minutes old.
_sccache_backing_off() {
  local failed_at="${1#failed:}"
  [[ "$1" == failed:* && "$failed_at" =~ ^[0-9]+$ ]] && ((${EPOCHSECONDS:-0} < failed_at + 300))
}

# Starts the server under a lock and records `ok`, or `failed:<epoch>` so
# shells retry at most once per five minutes. Re-reads the status under the
# lock: shells queued behind a failed start would otherwise each wait out
# their own 60-second attempt.
_sccache_start() {
  local state="$1"
  mkdir -p "$state" 2>/dev/null || return 0
  (
    flock -w 60 9 || exit 0
    status=""
    [ -s "$state/status" ] && read -r status <"$state/status"
    [ "$status" = ok ] && exit 0
    _sccache_backing_off "$status" && exit 0
    if timeout 60 sccache --start-server >"$state/err" 2>&1; then
      printf 'ok\n' >"$state/status"
    else
      printf 'failed:%s\n' "${EPOCHSECONDS:-0}" >"$state/status"
      echo "[sccache] could not start against s3://$SCCACHE_BUCKET/$SCCACHE_S3_KEY_PREFIX, compiling uncached:" >&2
      grep -m3 -i 'error' "$state/err" | cut -c1-400 | sed 's/^/[sccache]   /' >&2
    fi
  ) 9>"$state/lock"
}

_sccache_env

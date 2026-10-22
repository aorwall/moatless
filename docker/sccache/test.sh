#!/usr/bin/env bash
# Tests docker/sccache/sccache-env.sh against a fake `sccache` on PATH.
set -euo pipefail
# A socket on stdin sends the `env -i` shells down bash's rshd path, which
# skips BASH_ENV (Dockerfile.sandbox-base); Claude Code's Bash tool hands one.
exec </dev/null

script="$(cd "$(dirname "$0")" && pwd)/sccache-env.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# A fake sccache: `--start-server` counts itself in $FAKE_STARTS and records
# the bucket it saw, or fails like an unreachable bucket when $FAKE_FAIL is set,
# after a second when $FAKE_SLOW is set.
# Without `-u BASH_ENV` the fake sources the script under test and waits on the
# lock its caller holds.
mkdir -p "$work/bin"
cat > "$work/bin/sccache" <<'SCCACHE'
#!/usr/bin/env -S -u BASH_ENV bash
[ "$1" = --start-server ] || exit 0
echo x >> "$FAKE_STARTS"
[ -z "${FAKE_SLOW:-}" ] || sleep 1
printf '%s %s %s\n' "$SCCACHE_BUCKET" "$SCCACHE_S3_KEY_PREFIX" "$SCCACHE_IDLE_TIMEOUT" > "$FAKE_SEEN"
if [ -n "${FAKE_FAIL:-}" ]; then
  echo 'sccache: error: cache storage failed to read: PermissionDenied (permanent)' >&2
  exit 2
fi
echo 'sccache: Starting the server...'
SCCACHE
chmod +x "$work/bin/sccache"

# Runs `bash -c "$1"` the way an agent's Bash tool does: BASH_ENV only.
run() {
  env -i PATH="$work/bin:/usr/bin:/bin" HOME="$work/home" TMPDIR="$work/tmp" BASH_ENV="$script" \
    SCCACHE_S3_BUCKET="${BUCKET-ci-sccache}" AWS_REGION=eu-north-1 \
    FAKE_STARTS="$work/starts" FAKE_SEEN="$work/seen" FAKE_FAIL="${FAKE_FAIL:-}" FAKE_SLOW="${FAKE_SLOW:-}" bash -c "$1"
}
starts() { if [ -e "$work/starts" ]; then wc -l < "$work/starts"; else echo 0; fi; }
reset() { rm -rf "$work/home" "$work/tmp" "$work/starts" "$work/seen"; mkdir -p "$work/home" "$work/tmp"; }
status_file() { echo "$work/tmp/sccache-env-$(id -u)/status"; }

# An unset bucket wraps nothing and starts nothing.
reset
out="$(BUCKET= run 'printf %s "${RUSTC_WRAPPER-unset}"')"
[ "$out" = unset ] || fail "unset bucket exported RUSTC_WRAPPER='$out'"
[ "$(starts)" -eq 0 ] || fail "unset bucket started a server"

# The first bash starts the server with the bucket and wraps rustc.
reset
out="$(run 'printf %s "$RUSTC_WRAPPER $SCCACHE_BUCKET $SCCACHE_REGION"')"
[ "$out" = "sccache ci-sccache eu-north-1" ] || fail "first bash exported '$out'"
[ "$(cat "$work/seen")" = "ci-sccache moatless/ 0" ] || fail "server started with '$(cat "$work/seen")'"

# A later bash wraps rustc without starting again, and prints nothing.
out="$(run 'echo ok' 2>&1)"
[ "$out" = ok ] || fail "started path printed '$out'"
[ "$(starts)" -eq 1 ] || fail "started path started again ($(starts) starts)"

# A failed start wraps nothing, warns on stderr only, and waits five minutes.
reset
out="$(FAKE_FAIL=1 run 'printf %s "${RUSTC_WRAPPER-unset}"' 2>"$work/err")"
[ "$out" = unset ] || fail "failed start exported RUSTC_WRAPPER='$out'"
grep -q 'PermissionDenied' "$work/err" || fail "failed start printed no error: $(cat "$work/err")"
FAKE_FAIL=1 run 'true' 2>/dev/null
[ "$(starts)" -eq 1 ] || fail "failed start retried at once ($(starts) starts)"

# Shells queued on the lock behind a failed start do not each retry it.
reset
for _ in 1 2 3; do FAKE_FAIL=1 FAKE_SLOW=1 run 'true' 2>/dev/null & done
wait
[ "$(starts)" -eq 1 ] || fail "concurrent shells after a failure started $(starts) times"

# Five minutes after a failure the next bash retries, and a success wraps.
printf 'failed:%s\n' "$(($(date +%s) - 301))" > "$(status_file)"
out="$(run 'printf %s "${RUSTC_WRAPPER-unset}"')"
[ "$out" = sccache ] || fail "retry after a failure exported '$out'"
[ "$(starts)" -eq 2 ] || fail "retry after a failure did not start ($(starts) starts)"

# A status file that does not parse is retried, not trusted.
reset
mkdir -p "$(dirname "$(status_file)")"
echo 'failed:junk' > "$(status_file)"
out="$(run 'printf %s "${RUSTC_WRAPPER-unset}"')"
[ "$out" = sccache ] || fail "unparsed status exported '$out'"

# A caller running under `set -eu` survives sourcing it, either way.
for fake_fail in "" 1; do
  reset
  out="$(env -i PATH="$work/bin:/usr/bin:/bin" HOME="$work/home" TMPDIR="$work/tmp" BASH_ENV="$script" \
    SCCACHE_S3_BUCKET=ci-sccache FAKE_STARTS="$work/starts" FAKE_SEEN="$work/seen" \
    FAKE_FAIL="$fake_fail" bash -euc 'echo ok' 2>/dev/null)"
  [ "$out" = ok ] || fail "set -eu caller (FAKE_FAIL='$fake_fail') printed '$out'"
done

[ "$failures" -eq 0 ] && echo "sccache-env: all tests passed"
exit "$failures"

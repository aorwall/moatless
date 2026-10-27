#!/usr/bin/env bash
# Tests docker/codeartifact/codeartifact-env.sh against a fake `aws` on PATH.
set -euo pipefail
# A socket on stdin sends the `env -i` shells down bash's rshd path, which
# skips BASH_ENV (Dockerfile.sandbox-base); Claude Code's Bash tool hands one.
exec </dev/null

script="$(cd "$(dirname "$0")" && pwd)/codeartifact-env.sh"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
failures=0

fail() { echo "FAIL: $*" >&2; failures=$((failures + 1)); }

# A fake aws CLI: prints $FAKE_TOKEN and $FAKE_EXPIRATION, or fails when
# $FAKE_FAIL is set, and counts its mints in $work/mints.
mkdir -p "$work/bin"
cat > "$work/bin/aws" <<'AWS'
#!/usr/bin/env bash
case "$1 $2" in
  "codeartifact get-authorization-token")
    echo x >> "$FAKE_MINTS"
    if [ -n "${FAKE_FAIL:-}" ]; then
      echo "An error occurred (AccessDeniedException): not authorized" >&2
      exit 254
    fi
    printf '%s\t%s\n' "$FAKE_TOKEN" "$FAKE_EXPIRATION" ;;
  "sts get-caller-identity") echo "arn:aws:sts::1:assumed-role/test/x" ;;
  *) exit 1 ;;
esac
AWS
chmod +x "$work/bin/aws"

# Runs `bash -c "$1"` the way an agent's Bash tool does: BASH_ENV only.
run() {
  env -i PATH="$work/bin:/usr/bin:/bin" HOME="$work/home" BASH_ENV="$script" \
    CODEARTIFACT_DOMAIN="${DOMAIN-soaplabs}" AWS_REGION=eu-north-1 \
    FAKE_MINTS="$work/mints" FAKE_TOKEN="${FAKE_TOKEN:-tok-1}" \
    FAKE_EXPIRATION="${FAKE_EXPIRATION:-$(date -u -d '+12 hours' +%FT%T+00:00)}" \
    FAKE_FAIL="${FAKE_FAIL:-}" bash -c "$1"
}
mints() { if [ -e "$work/mints" ]; then wc -l < "$work/mints"; else echo 0; fi; }
reset() { rm -rf "$work/home" "$work/mints"; mkdir -p "$work/home"; }
host="soaplabs-222222222222.d.codeartifact.eu-north-1.amazonaws.com"

# An unset domain configures nothing and mints nothing.
reset
out="$(DOMAIN= run 'printf %s "${CODEARTIFACT_AUTH_TOKEN-unset}"')"
[ "$out" = unset ] || fail "unset domain exported '$out'"
[ ! -e "$work/home/.npmrc" ] || fail "unset domain wrote ~/.npmrc"
[ "$(mints)" -eq 0 ] || fail "unset domain minted"

# The first bash mints, exports the token and writes npm and pip config.
reset
out="$(run 'printf %s "$CODEARTIFACT_AUTH_TOKEN"')"
[ "$out" = tok-1 ] || fail "first bash exported '$out'"
expected="//$host/npm/npm/:_authToken=tok-1
//$host/npm/npm/:always-auth=true
//$host/npm/npm-prerelease/:_authToken=tok-1
//$host/npm/npm-prerelease/:always-auth=true"
[ "$(cat "$work/home/.npmrc")" = "$expected" ] || fail "~/.npmrc: $(cat "$work/home/.npmrc")"
grep -qx "index-url = https://aws:tok-1@$host/pypi/pypi/simple/" "$work/home/.config/pip/pip.conf" \
  || fail "pip.conf: $(cat "$work/home/.config/pip/pip.conf")"
for f in .npmrc .config/pip/pip.conf .cache/codeartifact/token; do
  [ "$(stat -c %a "$work/home/$f")" = 600 ] || fail "$f is not mode 600"
done

# A later bash reuses the cached token, prints nothing, and does not mint.
out="$(run 'echo ok' 2>&1)"
[ "$out" = ok ] || fail "cached path printed '$out'"
[ "$(mints)" -eq 1 ] || fail "cached path minted again ($(mints) mints)"

# Ten minutes before expiry the next bash renews and rewrites the config.
reset
FAKE_EXPIRATION="$(date -u -d '+5 minutes' +%FT%T+00:00)" run 'true'
out="$(FAKE_TOKEN=tok-2 run 'printf %s "$CODEARTIFACT_AUTH_TOKEN"')"
[ "$out" = tok-2 ] || fail "renewal exported '$out'"
grep -q '_authToken=tok-2' "$work/home/.npmrc" || fail "renewal left ~/.npmrc stale"

# A token /etc/mavenrc minted into the shared cache reaches ~/.npmrc too.
reset
run 'true'
printf 'tok-maven\n' > "$work/home/.cache/codeartifact/token"
date -d '+11 hours' +%s > "$work/home/.cache/codeartifact/token.exp"
out="$(run 'printf %s "$CODEARTIFACT_AUTH_TOKEN"')"
[ "$out" = tok-maven ] || fail "mavenrc token exported '$out'"
grep -q '_authToken=tok-maven' "$work/home/.npmrc" || fail "mavenrc token missed ~/.npmrc"

# A failed first mint leaves a commented diagnosis in ~/.npmrc, warns on
# stderr only, and waits five minutes before calling aws again.
reset
out="$(FAKE_FAIL=1 run 'printf %s "${CODEARTIFACT_AUTH_TOKEN-unset}"' 2>"$work/err")"
[ "$out" = unset ] || fail "failed mint exported '$out'"
grep -q 'AccessDeniedException' "$work/err" || fail "failed mint printed no error"
grep -vq '^#' "$work/home/.npmrc" && fail "failure ~/.npmrc has a non-comment line"
grep -q 'AccessDeniedException' "$work/home/.npmrc" || fail "failure ~/.npmrc lacks the error"
grep -q 'assumed-role/test' "$work/home/.npmrc" || fail "failure ~/.npmrc lacks the identity"
FAKE_FAIL=1 run 'true' 2>/dev/null
[ "$(mints)" -eq 1 ] || fail "failed mint retried at once ($(mints) mints)"

# A failed renewal keeps a still-valid token and its config.
reset
FAKE_EXPIRATION="$(date -u -d '+5 minutes' +%FT%T+00:00)" run 'true'
out="$(FAKE_FAIL=1 run 'printf %s "$CODEARTIFACT_AUTH_TOKEN"' 2>/dev/null)"
[ "$out" = tok-1 ] || fail "failed renewal exported '$out'"
grep -q '_authToken=tok-1' "$work/home/.npmrc" || fail "failed renewal replaced ~/.npmrc"

# An expiration that does not parse is taken as the 12-hour default.
reset
FAKE_EXPIRATION=None run 'true'
exp="$(cat "$work/home/.cache/codeartifact/token.exp")"
[ "$exp" -gt "$(date -d '+11 hours' +%s)" ] || fail "unparsed expiration cached as '$exp'"

# A caller running under `set -eu` survives sourcing it.
reset
out="$(env -i PATH="$work/bin:/usr/bin:/bin" HOME="$work/home" BASH_ENV="$script" \
  CODEARTIFACT_DOMAIN=soaplabs FAKE_MINTS="$work/mints" FAKE_TOKEN=tok-1 FAKE_EXPIRATION=None \
  bash -euc 'echo ok')"
[ "$out" = ok ] || fail "set -eu caller printed '$out'"

# /etc/mavenrc shares the cache: its mint lands whole, and its failed mint
# keeps a token this script still holds as valid.
mavenrc="$(dirname "$script")/../features/java/mavenrc"
maven() {
  env -i PATH="$work/bin:/usr/bin:/bin" HOME="$work/home" CODEARTIFACT_DOMAIN=soaplabs \
    FAKE_MINTS="$work/mints" FAKE_TOKEN="${FAKE_TOKEN:-tok-1}" FAKE_FAIL="${FAKE_FAIL:-}" \
    FAKE_EXPIRATION="$(date -u -d '+12 hours' +%FT%T+00:00)" sh -c ". '$mavenrc'" 2>/dev/null
}
reset
FAKE_TOKEN=tok-mvn maven
[ "$(cat "$work/home/.cache/codeartifact/token")" = tok-mvn ] || fail "mavenrc mint not cached"
printf '%s\n' "$(date -d '+5 minutes' +%s)" > "$work/home/.cache/codeartifact/token.exp"
FAKE_FAIL=1 maven
out="$(FAKE_FAIL=1 run 'printf %s "${CODEARTIFACT_AUTH_TOKEN-unset}"' 2>/dev/null)"
[ "$out" = tok-mvn ] || fail "failed mavenrc mint dropped a valid token: '$out'"
grep -q '_authToken=tok-mvn' "$work/home/.npmrc" || fail "failed mavenrc mint cost ~/.npmrc its token"

[ "$failures" -eq 0 ] && echo "codeartifact-env: all tests passed"
exit "$failures"

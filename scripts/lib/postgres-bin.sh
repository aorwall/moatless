# Sourced by scripts that start a throwaway Postgres; the caller defines `log`.

# Postgres binaries for when none is installed; npm serves them.
EMBEDDED_PG_VERSION="16.15.0-beta.19"

# A directory holding `initdb`, `pg_ctl` and `postgres`: LOCAL_E2E_PG_BIN, an
# installed server, or the embedded-postgres npm package fetched into a cache.
pg_bin_dir() {
  local dir
  for dir in "${LOCAL_E2E_PG_BIN:-}" "$(dirname "$(command -v pg_ctl 2>/dev/null || echo /nonexistent/x)")" /usr/lib/postgresql/*/bin; do
    if [ -n "$dir" ] && [ -x "$dir/postgres" ] && [ -x "$dir/initdb" ]; then
      echo "$dir"
      return
    fi
  done
  local os arch pkg cache
  case "$(uname -s)" in Linux) os=linux ;; Darwin) os=darwin ;; *) log "no Postgres build for $(uname -s)"; exit 1 ;; esac
  case "$(uname -m)" in x86_64|amd64) arch=x64 ;; aarch64|arm64) arch=arm64 ;; *) log "no Postgres build for $(uname -m)"; exit 1 ;; esac
  pkg="@embedded-postgres/${os}-${arch}"
  cache="${XDG_CACHE_HOME:-$HOME/.cache}/moatless-local-e2e/pg-${EMBEDDED_PG_VERSION}"
  if [ ! -x "${cache}/node_modules/${pkg}/native/bin/postgres" ]; then
    log "Fetching ${pkg}@${EMBEDDED_PG_VERSION} into ${cache}"
    npm install --prefix "$cache" "${pkg}@${EMBEDDED_PG_VERSION}" --no-audit --no-fund --silent >&2
    # The tarball ships the shared-library symlinks as a list; initdb needs them.
    (cd "${cache}/node_modules/${pkg}" && node scripts/hydrate-symlinks.js) >&2
  fi
  echo "${cache}/node_modules/${pkg}/native/bin"
}

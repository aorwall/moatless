# AGENTS.md — docker/

## Image graph

`ci.yml` builds every image and pushes it to Docker Hub under `REGISTRY_ORG`
(repo variable, default `aorwall`). The cluster pulls from there.

```
Dockerfile.sandbox-base ──> aorwall/moatless-sandbox-base ──┬─> Dockerfile.sandbox      (+ sandbox-js alias)
                                                            ├─> recipes (docker/recipes/*.json, build-sandboxes.yml)
                                                            ├─> Dockerfile.sandbox-dev  (build-dev-image.yml, manual)
                                                            └─> Dockerfile.sandbox-full / -e2e / -crash (local)
Dockerfile.sandbox-tools ─> entrypoint + moatless-git-cred, consumed by COPY --from
Dockerfile.app ──────────> moatless-frontend
Dockerfile.backend-cross ─> moatless-backend
```

Everything downstream of `sandbox-base` is a thin overlay. Install a tool in
`sandbox-base`; do not reinstall it in a child image.

## Layer order in Dockerfile.sandbox-base

**Order layers volatile-last. The Claude and Codex CLI layers stay at the
bottom, one CLI per layer, and nothing goes below them.**

The image is ~2 GB over 26 layers and the two CLI pins in `cli-versions.env` are
Renovate-driven and by far the most frequent reason it rebuilds. When those
layers sat in the middle, one CLI bump invalidated 1.23 GB — 60% of the image,
including Playwright's 467 MB and Vite+'s 186 MB — none of which depends on
them. Adding a new tool means adding it *above* the CLI layers.

`APT_SECURITY_REFRESH` is the opposite case: it is near the top and invalidates
everything, which is the point — it exists to force a full rebuild.

## Build cache

**Every image exports to a registry cache — `<image>:buildcache` — never
`type=gha`.** `image-manifest=true,oci-mediatypes=true` pushes the cache as an
ordinary image manifest, which Docker Hub accepts. `sandbox-base` uses
`mode=max`; the leaf images use `mode=min`.

Two separate reasons, and the second is the one that generalises:

`sandbox-base`'s `mode=max` export alone was ~7 GB against the repository's
10 GB Actions-cache budget, so it evicted its own previous entry between runs —
a 0% layer hit rate on a build whose inputs were byte-identical, and 606s of an
840s build spent writing a cache nothing could read.

Moving it was not enough. The five remaining `type=gha` images refilled the
space it freed: `buildkit-blob` still held **7.29 GB across 252 entries** — 70%
of the budget, on a repo already 0.34 GB over cap. The budget is shared and
LRU, and a small writer with a high run rate crowds out a large reader just as
effectively as a large writer does. These builds are all under a minute; the
Rust target-dir caches they compete with are what gates PR wall clock. Docker
Hub has no such cap, so nothing in `ci.yml` should hold Actions-cache space
except the Rust target dirs and the per-run binary handoffs.

The first build after adding a `:buildcache` ref logs a `cache-from` miss. That
is the tag not existing yet, not a failure; the run after it hits.

`RUN --mount=type=cache` is worth nothing here: every job gets a fresh
`docker-container` buildx driver, and cache mounts are builder-local and not
exported by `--cache-to`.

## npm global symlinks

Link into `/usr/local/bin` from `$(npm prefix -g)/bin/<tool>`, never from
`$(which <tool>)`. `/usr/local/bin` precedes npm's bin dir on `PATH`, so once
the symlink exists `which` resolves to it and the link points at itself —
ELOOP ("Too many levels of symbolic links") the next time anything runs the
tool.

## Version pins

`docker/cli-versions.env` is the single source of truth for the tool versions
every build path passes as `--build-arg`; the `ARG` defaults in the Dockerfiles
are inert fallbacks for a bare `docker build`. Bump versions there, and keep the
`# renovate:` annotation on the line directly above each pin. A pin nothing
consumes still triggers a full rebuild when Renovate bumps it — delete the pin
when you delete its consumer.

`RUST_VERSION` in `Dockerfile.sandbox-base` must match `rust-toolchain.toml`;
the `toolchain-drift` CI job fails when they diverge.

## Recipes

A recipe (`docker/recipes/<name>.json`) composes a base (`sandbox` or
`sandbox-base`) with features (`docker/features/<id>/{feature.json,install.sh}`).
`docker/generate-recipe.sh` turns one into a Dockerfile and a content-hash tag;
`build-sandboxes.yml` builds only the recipes whose tag doesn't already
exist in the registry and pushes both the immutable `:<hash12>` tag and a
moving `:<recipe-filename>` alias (e.g. `aorwall/moatless-sandbox-recipe:soaplabs`).
It is triggered by `ci.yml`'s completion (`workflow_run`, not an independent
`push` trigger) so recipe discovery only ever runs after `ci.yml` has actually
published a new `sandbox`/`sandbox-base` image — see the comment at the top of
the workflow file for why an independent push trigger raced it.

**A feature installs a tool and, if the sandbox process needs to see it at
runtime, the environment it requires.** `install.sh` itself can't set `ENV` —
it runs inside a `RUN`'d shell — so a feature declares runtime env vars in its
`feature.json`'s `env` map instead; `generate-recipe.sh` emits them as real
Dockerfile `ENV` instructions right after that feature's `RUN` step. No
entrypoint changes beyond that — the base image (`sandbox-base`) still owns
everything not specific to the feature's own tool.

**The recipe's own JSON is not enough to make the tag content-addressed.**
`generate-recipe.sh` folds in a hash of each feature's files and the resolved
digest of the base, so an `install.sh` fix or a `sandbox`/`sandbox-base`
rebuild produces a new tag instead of silently being skipped as "already
built". See `.plans/custom-sandbox-images.md`.

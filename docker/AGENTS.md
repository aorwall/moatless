# AGENTS.md — docker/

## Image graph

`ci.yml` builds every image and pushes it to Docker Hub under `REGISTRY_ORG`
(repo variable, default `aorwall`). The cluster pulls from there.

```
Dockerfile.sandbox-base ──> aorwall/moatless-sandbox-base ──┬─> Dockerfile.sandbox      (+ sandbox-js alias)
                                                            ├─> Dockerfile.sandbox-java (matrix: 21, 25)
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

`sandbox-base` exports to a registry cache
(`<org>/moatless-sandbox-base:buildcache`), not `type=gha`. Its `mode=max` export
is ~7 GB against the repository's 10 GB Actions-cache budget, so it evicted its
own previous entry — and the Rust caches — between runs, and measured a 0% layer
hit rate on a build whose inputs were byte-identical, while spending 606s of an
840s build writing the cache. Docker Hub has no such cap.
`image-manifest=true,oci-mediatypes=true` pushes the cache as an ordinary image
manifest, which Docker Hub accepts.

The other images stay on `type=gha,mode=min`; they are small enough not to
crowd out the Rust caches. If one starts to, move it the same way.

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

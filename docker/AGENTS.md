# docker/

## Building an image locally

```bash
./docker/build-backend.sh    # each takes --local to skip the push
./docker/build-sandbox.sh    # on Apple Silicon: --local --only base
```

## How the images depend on each other

`ci.yml` builds every image and pushes it to Docker Hub under `REGISTRY_ORG`, a
repo variable that defaults to `aorwall`. The cluster pulls from there.

```
Dockerfile.sandbox-base ──> aorwall/moatless-sandbox-base ──┬─> Dockerfile.sandbox      (+ sandbox-js alias)
                                                            ├─> recipes (docker/recipes/*.json, build-sandboxes.yml)
                                                            └─> Dockerfile.sandbox-e2e (local)
                                    aorwall/moatless-sandbox ──> Dockerfile.sandbox-dev  (build-dev-image.yml, manual)
Dockerfile.sandbox-tools ─> entrypoint + git helpers, consumed by COPY --from
Dockerfile.sandbox-crash ─> ubuntu:24.04 direct, in no subtree
Dockerfile.backend-cross ─> moatless-backend
```

Everything downstream of `sandbox-base` is a thin overlay: install a tool in
`sandbox-base`, never again in a child image. `sandbox-dev` descends from
`moatless-sandbox` rather than the base, so a tool added to `sandbox-base`
reaches it only after `sandbox` rebuilds. A language toolchain is the
exception: the base carries Node, Bun, Python and Rust and nothing else, and a
JDK, Maven and `psql` come from `docker/recipes/java.json`. A Java repo
reaches them only by setting its workspace `dockerImage` to
`<REGISTRY_ORG>/moatless-sandbox-recipe:java`.

## Deciding whether the sandbox binaries changed

**Run `task sandbox:rebuild-needed [BASE=<rev>]`** to learn whether the commits
since `BASE` need new `sandbox` and `moat` binaries; `ci.yml`'s `detect-changes`
gates both builds on the same script. A file a closure crate reads other than
through a literal `include_str!`, `include_bytes!` or `#[path]` goes in the
script's `ALWAYS`.

## Ordering layers in Dockerfile.sandbox-base

**Order layers volatile-last. Keep the Claude and Codex CLI layers at the
bottom, one CLI per layer, with nothing below them. Add a new tool above
them.**

The two CLI pins in `cli-versions.env` are bumped daily and are the most
frequent reason the 2 GB image rebuilds; a CLI layer in the middle invalidates
1.23 GB below it — Playwright's 467 MB and Vite+'s 186 MB among them, and
neither depends on either CLI.

`APT_SECURITY_REFRESH` is the opposite case. It sits near the top and
invalidates everything, which is what it exists to do: force a full rebuild.

## Exporting the build cache

**Export every image to a registry cache at `<image>:buildcache` for AMD64 or
`<image>:buildcache-arm64` for ARM64. Never export to `type=gha`.**
Docker Hub accepts caches with `image-manifest=true,oci-mediatypes=true`.
Use `mode=max` for `sandbox-base` and `mode=min` for leaf images.

`type=gha` breaks because `sandbox-base`'s 7 GB export evicts its own previous
entry from the 10 GB shared, LRU Actions cache, so it never hits. Only the Rust
target dirs and per-run binary handoffs belong in that cache.

`RUN --mount=type=cache` is worth nothing here. Every job gets a fresh
`docker-container` buildx driver, and a cache mount is builder-local and
`--cache-to` does not export it.

## Installing an npm global tool

**Give every `RUN` that calls `npm` a `--mount=type=tmpfs,target=/root/.npm`.**
npm caches downloads there even for `-g`, and no later `rm -rf` shrinks a layer.

Link into `/usr/local/bin` from `$(npm prefix -g)/bin/<tool>`. Never link from
`$(which <tool>)`. `/usr/local/bin` precedes npm's bin dir on `PATH`, so once
the symlink exists `which` resolves to it and the link points at itself. The
next run of the tool then fails with ELOOP, "Too many levels of symbolic
links".

## Bumping a version pin

`docker/cli-versions.env` documents where a pin is consumed and how to bump it.
Delete a pin when you delete its consumer: a pin nothing consumes still triggers
a full rebuild when it is bumped.

**The `agent-cli-upgrade` loop bumps `CLAUDE_CLI_VERSION` and
`CODEX_CLI_VERSION`** (`.moatless/loops/agent-cli-upgrade/LOOP.md`). The
`# renovate:` annotations never run: Renovate is not onboarded here. Do not hand
it the two pins: its Monday schedule and three-day `minimumReleaseAge` land a
CLI up to ten days late, and skip the loop's `agent_transport_e2e.rs` check.

A hand bump moves every file the loop file lists, five for Claude and two for
Codex: `crates/claude-agent-sdk`'s smoke test fails on a partial bump.

## Building a recipe

A recipe (`docker/recipes/<name>.json`) composes a base, `sandbox` or
`sandbox-base`, with features
(`docker/features/<id>/{feature.json,install.sh}`). `docker/generate-recipe.sh`
turns one recipe into a Dockerfile and a content-hash tag.

`build-sandboxes.yml` builds only the recipes whose tag is absent from the
registry, pushing both the immutable `:<hash12>` tag and a moving
`:<recipe-filename>` alias. Trigger it from `ci.yml`'s completion through
`workflow_run`, never through an independent `push` trigger, which races the
`sandbox` and `sandbox-base` publish.

**A feature installs a tool. When the sandbox process must see that tool at
runtime, the feature also declares the environment it requires.** `install.sh`
cannot set `ENV`, because it runs inside a `RUN` shell. Declare a runtime env
var in the `env` map of the feature's `feature.json` instead, and
`generate-recipe.sh` emits it as a real `ENV` instruction after that feature's
`RUN` step. Change nothing in the entrypoint.

**The recipe's own JSON is not enough to make the tag content-addressed.**
`generate-recipe.sh` folds in a hash of each feature's files and the resolved
digest of the base. An `install.sh` fix or a base rebuild then produces a new
tag, instead of a silent skip as "already built".

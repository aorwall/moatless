# docker/

## Building an image locally

```bash
./docker/build-app.sh        # frontend nginx image
./docker/build-backend.sh
./docker/build-sandbox.sh    # on Apple Silicon: --local --only base
```

Each takes `--local` to skip the push.

## How the images depend on each other

`ci.yml` builds every image and pushes it to Docker Hub under `REGISTRY_ORG`, a
repo variable that defaults to `aorwall`. The cluster pulls from there.

```
Dockerfile.sandbox-base ──> aorwall/moatless-sandbox-base ──┬─> Dockerfile.sandbox      (+ sandbox-js alias)
                                                            ├─> recipes (docker/recipes/*.json, build-sandboxes.yml)
                                                            └─> Dockerfile.sandbox-full / -e2e (local)
                                    aorwall/moatless-sandbox ──> Dockerfile.sandbox-dev  (build-dev-image.yml, manual)
Dockerfile.sandbox-tools ─> entrypoint + moatless-git-cred, consumed by COPY --from
Dockerfile.sandbox-crash ─> ubuntu:24.04 direct, in no subtree
Dockerfile.app ──────────> moatless-frontend
Dockerfile.backend-cross ─> moatless-backend
```

`sandbox-dev` descends from `moatless-sandbox`, not from the base, so a tool
added to `sandbox-base` reaches it only after `sandbox` rebuilds.

Everything downstream of `sandbox-base` is a thin overlay. Install a tool in
`sandbox-base`. Do not reinstall it in a child image.

## Ordering layers in Dockerfile.sandbox-base

**Order layers volatile-last. Keep the Claude and Codex CLI layers at the
bottom, one CLI per layer, with nothing below them. Add a new tool above
them.**

The image is about 2 GB over 26 layers. The two CLI pins in `cli-versions.env`
are bumped daily and are by far the most frequent reason it rebuilds. A CLI
layer placed in the middle invalidates 1.23 GB on one bump, which is 60% of the
image. That includes Playwright's 467 MB and Vite+'s 186 MB, and neither
depends on either CLI.

`APT_SECURITY_REFRESH` is the opposite case. It sits near the top and
invalidates everything, which is what it exists to do: force a full rebuild.

## Exporting the build cache

**Export every image to a registry cache at `<image>:buildcache`. Never export
to `type=gha`.** `image-manifest=true,oci-mediatypes=true` pushes the cache as
an ordinary image manifest, which Docker Hub accepts. `sandbox-base` uses
`mode=max`, and the leaf images use `mode=min`.

Two measurements set that rule.

`sandbox-base`'s `mode=max` export is about 7 GB against the repository's 10 GB
Actions-cache budget. In that budget it evicts its own previous entry between
runs. The layer hit rate then falls to 0% on a build whose inputs are
byte-identical. 606s of an 840s build goes on writing a cache nothing can read.

Moving one image is not enough. Five `type=gha` images refilled the space that
move freed. `buildkit-blob` then held 7.29 GB across 252 entries: 70% of the
budget, on a repo already 0.34 GB over cap. The budget is shared and LRU, so a
small writer with a high run rate evicts as much as a large writer does. These
image builds each take under a minute, and the Rust target-dir caches they
compete with are what gates PR wall clock. Docker Hub has no such cap. Nothing
in `ci.yml` should hold Actions-cache space except the Rust target dirs and the
per-run binary handoffs.

The first build after you add a `:buildcache` ref logs a `cache-from` miss. The
tag does not exist yet, and the run after it hits.

`RUN --mount=type=cache` is worth nothing here. Every job gets a fresh
`docker-container` buildx driver, and a cache mount is builder-local and
`--cache-to` does not export it.

## Linking an npm global tool

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
`CODEX_CLI_VERSION`** (`.moatless/loops/agent-cli-upgrade/LOOP.md`). Renovate
does not. The `# renovate:` customManager in `renovate.json` is written and
correct. The bot is not onboarded on this repository, so the annotations
describe a backstop that does not run.

Renovate would not serve those two pins even once onboarded. The shared preset
(`soaplabs/renovate-config`) schedules Mondays with a three-day
`minimumReleaseAge`, and `soaplabs/moatless` is absent from its auto-merge
repository list. A CLI that ships several releases a week would land up to ten
days late, behind a manual merge. The loop runs daily, and it drives the
candidate CLIs through `sandbox/tests/agent_transport_e2e.rs` before it opens
the PR.

A Claude bump moves five files and a Codex bump moves two. The loop file lists
them. Bump all of them when you bump by hand: `crates/claude-agent-sdk`'s smoke
test asserts the version and fails on a partial bump.

## Building a recipe

A recipe (`docker/recipes/<name>.json`) composes a base, `sandbox` or
`sandbox-base`, with features
(`docker/features/<id>/{feature.json,install.sh}`). `docker/generate-recipe.sh`
turns one recipe into a Dockerfile and a content-hash tag.

`build-sandboxes.yml` builds only the recipes whose tag is absent from the
registry. It pushes both the immutable `:<hash12>` tag and a moving
`:<recipe-filename>` alias, such as `aorwall/moatless-sandbox-recipe:soaplabs`.
`ci.yml`'s completion triggers it, through `workflow_run` and never through an
independent `push` trigger. Recipe discovery then runs only after `ci.yml` has
published a new `sandbox` or `sandbox-base` image. The comment at the top of
the workflow file records how an independent push trigger raced it.

**A feature installs a tool. When the sandbox process must see that tool at
runtime, the feature also declares the environment it requires.** `install.sh`
cannot set `ENV`, because it runs inside a `RUN` shell. Declare a runtime env
var in the `env` map of the feature's `feature.json` instead.
`generate-recipe.sh` emits it as a real Dockerfile `ENV` instruction, directly
after that feature's `RUN` step. Change nothing in the entrypoint beyond that.
The base image `sandbox-base` still owns everything that is not specific to the
feature's own tool.

**The recipe's own JSON is not enough to make the tag content-addressed.**
`generate-recipe.sh` folds in a hash of each feature's files and the resolved
digest of the base. An `install.sh` fix or a base rebuild then produces a new
tag, instead of a silent skip as "already built". See
`.plans/custom-sandbox-images.md`.

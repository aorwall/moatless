# Custom sandbox images

> **Status:** proposed
> **Updated:** 2026-08-05
> **Vocabulary:** CONTEXT.md

## Progress

- **2026-08-12 — soaplabs recipe shipped.** `docker/features/java`,
  `docker/features/postgres-client`, `docker/recipes/soaplabs.json` and
  `build-sandboxes.yml` replaced `Dockerfile.sandbox-java` and its CI matrix
  job. One recipe, not a generic `java-21`/`java-25` pair: nothing needed
  Java on its own in this fork. `postgresql-client` is its own feature rather
  than folded into `java`'s `install.sh` — it's a separate tool with its own
  apt package, useful to any recipe regardless of whether it also wants Java.
  This is *not* the phase 1 "definition of done" below (`moatless-fullstack`
  on a recipe, `Dockerfile.sandbox-full` deleted) — soaplabs was the concrete
  near-term need; fullstack wasn't (nothing in `.moatless/workspaces.json`
  references `sandbox-full` today). The recipe/feature machinery
  (`docker/features/`, `docker/recipes/`, `docker/generate-recipe.sh`, the
  discover/build workflow split) is the same machinery phase 1 describes;
  phase 1 itself remains open.
- AWS CLI was **not** moved out of `Dockerfile.sandbox-base` into a feature,
  despite looking soap-specific at first glance (it backs the CodeArtifact
  and sccache bootstrap hooks in `docker-entrypoint-sandbox.sh`). This repo's
  own default workspaces (`moatless`, `moatless-preview` in
  `.moatless/workspaces.json`) set `SCCACHE_S3_BUCKET` and run on the default
  `sandbox` image with no `dockerImage` override — extracting AWS CLI to a
  feature only Java recipes pull in silently broke sccache for every normal
  task on this repo. It stays fork-wide infrastructure in the base image.
- `build-sandboxes.yml` triggers on `workflow_run` off `ci.yml`'s completion
  rather than its own `push` + path filter, so recipe discovery always runs
  after `ci.yml` has actually published a new `sandbox`/`sandbox-base` image
  — an independent push trigger could otherwise resolve a recipe's base-digest
  pin against a stale `:latest` before `ci.yml`'s own publish jobs finished.
- A feature's `feature.json` can declare a static `env` map; `generate-recipe.sh`
  emits it as real Dockerfile `ENV` instructions after that feature's `RUN`
  step. `install.sh` can't set `ENV` itself (it runs inside a `RUN`'d shell),
  and the sandbox entrypoint execs the binary directly without sourcing
  `/etc/profile`, so `ENV` is what the running process actually inherits —
  an `/etc/profile` append alone (this doc's original sketch) is not enough.

## Current solution

A workspace picks its sandbox image by name, and that image must already exist.

- `Workspace.docker_image` (`backend/src/workspace/models.rs:23`) is the only
  runtime knob. It is set from the API/CLI (`crates/cli/src/commands/workspaces.rs:28`)
  or declared per entry as `dockerImage` in `.moatless/workspaces.json`
  (`backend/src/workspace/git_sync.rs:99`).
- It reaches the pod through `build_repo_config_from_workspace`
  (`backend/src/sandbox/k8s/auto_provision.rs:70`) and
  `resolve_sandbox_image` (`backend/src/sandbox/k8s/pod_builder.rs:383`, docker
  equivalent `backend/src/sandbox/docker/container_builder.rs:41`), falling back
  to `Config.sandbox.image` / `SANDBOX_IMAGE` (`backend/src/config.rs:1047`).
- **Nothing in the product builds an image.** There is no `docker build`,
  BuildKit or kaniko call anywhere in `backend/`, `sandbox/` or `crates/`. Every
  image is prebuilt out of band by `ci.yml` or `docker/build-sandbox.sh`.

So a workspace that needs a tool `sandbox-base` does not carry has two options,
and both are bad:

1. **A new hand-written Dockerfile variant.** This is why `docker/` holds
   `Dockerfile.sandbox-java` (a 21/25 build matrix), `Dockerfile.sandbox-full`
   (Python + Go + Java) and `Dockerfile.sandbox-dev`. Each is a separate file, a
   separate CI job, a separate cache scope, and a separate thing to keep in step
   with `sandbox-base`. The variants do not compose: there is no
   "Java 21 + Python" image and adding one means a fourth file.
2. **`setupCommands`** (`backend/src/workspace/git_sync.rs:74`, executed by
   `run_workspace_setup` at `sandbox/src/init/mod.rs:734`). This works and is
   what `moatless-fullstack` uses today (`.moatless/workspaces.json:78`), but it
   pays the install cost on **every sandbox start**, over the network, with no
   cache and no pinning.

The cost of option 1 is measured. `sandbox-base` is 2.03 GB over 26 layers and
takes 14 minutes to build; the child variants rebuild whenever it moves. The
cost of option 2 is a slower first prompt in every single task on that
workspace, forever.

## Concept: recipes

**A recipe is a declarative list of pinned features layered on a base image. Its
content hash is its tag.** Two workspaces asking for the same thing get the same
image, built once.

```jsonc
// .moatless/workspaces.json — a workspace entry
{
  "key": "moatless-fullstack",
  "recipe": {
    "base": "sandbox",                       // sandbox | sandbox-base
    "features": {
      "java":   { "version": "21" },
      "python": { "version": "3.13" }
    }
  }
}
```

resolves to `aorwall/moatless-sandbox-recipe:<sha256(recipe)[:12]>`, which the
backend puts in `Workspace.docker_image` exactly as a hand-written name today.
No change to `pod_builder.rs` or `container_builder.rs` — the recipe is resolved
one level above them, and the runtime keeps seeing a plain image reference.

A **feature** is a directory under `docker/features/<id>/`:

```
docker/features/java/
  feature.json     # id, description, option schema, installsAfter
  install.sh       # runs as root, receives options as env vars
```

The generated Dockerfile is deliberately dumb:

```dockerfile
FROM aorwall/moatless-sandbox:<pinned-digest>
COPY features/java/   /tmp/features/java/
RUN JAVA_VERSION=21 /tmp/features/java/install.sh && rm -rf /tmp/features/java
COPY features/python/ /tmp/features/python/
RUN PYTHON_VERSION=3.13 /tmp/features/python/install.sh && rm -rf /tmp/features/python
```

Three properties follow from this shape and they are the whole point:

- **Deterministic order** (topological by `installsAfter`, ties broken by id) means
  two recipes that share a prefix share layers in the registry cache. A
  `java+python` recipe and a `java+go` recipe build the second feature only.
- **Content-addressed tag** means a build is skipped entirely when the tag
  already exists, and a recipe change produces a new tag rather than mutating one
  — no `:latest` race, no stale pod pulling yesterday's image.
- **Pinned versions are part of the hash.** A floating version would make the
  hash lie about the contents, so recipes reject unpinned options. This is the
  one rule that cannot be relaxed later.

The rules in `docker/AGENTS.md` apply to generated Dockerfiles too: features go
above nothing (they are the volatile part by construction), and the recipe
builder uses a registry cache, never `type=gha`.

## Build path

**Phase 1 — CI builds recipes, backend only reads them.**

A `build-sandboxes.yml` workflow that:

1. Enumerates every recipe reachable from `.moatless/workspaces.json` across the
   synced repos, plus any recipe committed under `docker/recipes/*.json`.
2. Computes each tag and asks the registry whether it exists (`HEAD` on the
   manifest). Skips the ones that do — the steady state is that this workflow
   does nothing.
3. Builds the rest as a matrix, `cache-from`/`cache-to` a shared
   `:recipe-buildcache` registry ref so the shared prefixes actually hit.

Triggered on `ci.yml`'s completion (`workflow_run`, gated to successful pushes
to `main`), not its own path-filtered `push` — a recipe pins its base image by
resolved digest at generation time, so discovery has to run *after* `ci.yml`
has actually republished `sandbox`/`sandbox-base`, not in parallel with it.
Also `workflow_dispatch`. The backend does not participate: it resolves a
recipe to a tag and uses it, and a missing tag is a startup failure it reports
rather than repairs.

**Definition of done:** `moatless-fullstack` runs on a recipe image with its
`setupCommands` deleted, and `Dockerfile.sandbox-full` is gone.

**Phase 2 — fold the existing variants in.**

`sandbox-java:21`, `sandbox-java:25` and `sandbox-full` become recipes. Their
Dockerfiles and their CI jobs are deleted. `sandbox-dev` is the interesting case:
it is a heavy shadow-dev image built manually (`build-dev-image.yml`) and it
should stay hand-written until phase 1 has proven the cache-sharing claim.

**Definition of done:** `docker/` holds `sandbox-base`, `sandbox`,
`sandbox-tools`, `sandbox-dev`, the app/backend images, and nothing else.

**Phase 3 (only if phase 1 justifies it) — build on demand.**

A workspace declares a recipe that has never been built and the backend triggers
the build instead of failing. This needs a builder that outlives a job — a
BuildKit `Deployment` in-cluster with a PVC, addressed through the buildx
`remote` driver — plus a queue, a per-workspace concurrency cap, and a way to
show build progress on the task. It is a much larger change than phases 1-2 and
should not be committed to in advance.

Until then, the failure mode is explicit: an unresolvable recipe leaves
`docker_image` unset, the sandbox starts on the default image, and the task gets
an event naming the missing recipe. It must never hang or silently run the wrong
image.

## Prior art, and what we take from it

- **devcontainer Features** ([containers.dev](https://containers.dev/implementors/features/))
  — the closest thing to a standard: an id, an options schema, an `install.sh`,
  and `installsAfter` ordering. **Take the interface shape**, and stay close
  enough that a future importer of the public feature registry is possible.
  **Do not take the runtime**: the devcontainer CLI builds at container-create
  time, which is the per-start cost this whole design exists to remove.
- **Gitpod `workspace-images`** — a common base plus per-tool chunks composed by
  Dazzle, with per-branch caching (~1 hr first build, ~25 min after). The chunk
  decomposition is the right instinct and is what recipes copy. **Skip Dazzle
  itself**: it needs a local registry and custom layer-merge tooling, and plain
  ordered layers over a shared base with content-addressed tags recover most of
  the reuse for none of the machinery.
- **`devcontainers/images`** — manifest-driven, with a paginated matrix building
  100+ variants. Confirms the matrix-over-a-manifest build shape at a scale well
  beyond ours.
- **Nix** — the strongest reproducibility story by a wide margin, and the honest
  answer to "pinned versions are part of the hash". Rejected because it replaces
  the entire toolchain story rather than extending it, and nothing in this repo
  or in the projects it sandboxes uses it.

## Open questions

- **Who may define a recipe?** `.moatless/workspaces.json` is git-synced from a
  repo, so a recipe is an arbitrary `install.sh` chosen by whoever can push to
  it. Features living in *this* repo under `docker/features/` keeps the
  executable surface reviewed; letting a workspace point at its own install
  script does not. Start with the former.
- **Garbage collection.** Content-addressed tags accumulate one per distinct
  recipe forever. Needs a retention rule (untagged after N days with no
  workspace referencing it) before phase 2, not after.
- **Base image pinning.** A recipe pins its base by digest, so a `sandbox-base`
  rebuild does not silently change a recipe image — but then recipes go stale.
  Something has to re-resolve and rebuild them on a base bump; the cheapest
  version is a scheduled job that re-runs the phase 1 workflow with the new
  digest.
- **Does `setupCommands` survive?** It should, for genuinely per-repo work
  (`bun install` against a lockfile). The line to hold is that it installs
  *project dependencies*, never *tools*.

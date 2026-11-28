# Local cluster (Docker Desktop on macOS, k3d on Linux)

Follow [the local runbook](../../../../k8s/local/README.md) from top to bottom.
It replaces steps 1-5 of the skill: the committed
[values file](../../../../k8s/local/values-local.yaml) needs no `moat k8s init`,
domain, or TLS.

- Pass `--context` to every `kubectl` call and `--kube-context` to every `helm`
  call: `docker-desktop` on macOS, `k3d-moatless` on Linux. The current context
  may be a remote cluster.
- On Linux, create the cluster with the runbook's step 0, not with Docker
  Desktop for Linux or kind: neither sees host-built images or serves port 80
  without extra setup.
- Check for an earlier release with `helm list` before you install.
- Use the published `aorwall` backend, sandbox, and T3 images. The local
  runbook explains how to select a local build when developing an image.
- Ask the user before you prune Docker data, reset the database, or store one of
  their tokens in the cluster.
- Verify in a browser. `curl` succeeds while the UI cannot connect.
- The install is verified only when a task runs: the runbook's "First run"
  section, then [manual smoke](../../e2e-testing/references/manual-smoke.md).
  It needs the user's Claude and GitHub tokens, so ask for them, preferably
  saved by the user in the UI. A sandbox pod without a task proves neither the
  agent nor the preview.

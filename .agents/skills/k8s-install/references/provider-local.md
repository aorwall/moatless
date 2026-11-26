# Local Docker Desktop

Follow [the local runbook](../../../../k8s/local/README.md) from top to bottom.
It replaces steps 1-5 of the skill: the committed
[values file](../../../../k8s/local/values-local.yaml) needs no `moat k8s init`,
domain, or TLS.

- Pass `--context docker-desktop` to every `kubectl` call and
  `--kube-context docker-desktop` to every `helm` call. The current context may
  be a remote cluster.
- Check for an earlier release with `helm list` before you install.
- Build images with [build-local-images.sh](../scripts/build-local-images.sh).
- Ask the user before you prune Docker data, reset the database, or store one of
  their tokens in the cluster.
- Verify in a browser. `curl` succeeds while the UI cannot connect.

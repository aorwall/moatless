#!/bin/bash
set -e

# Image ENTRYPOINT for docker-compose and `docker run` only. Kubernetes never
# runs it: the pod `command` (sandbox-k8s pod_builder/containers.rs) replaces the
# ENTRYPOINT, so a step added here skips every Kubernetes sandbox. Put setup
# every launch needs in the image itself (an ENV, or a file under /etc such as
# /etc/codeartifact-env.sh) or in the sandbox binary.

# `gh` builds its git credential helper from `GH_PATH` (also exported by the
# sandbox binary, since K8s skips this script); moatless-git-cred
# answers that helper from the broker and hands every other call to `gh`.
if [ -x /usr/local/bin/moatless-git-cred ]; then
    export GH_PATH=/usr/local/bin/moatless-git-cred
fi

exec /usr/local/bin/sandbox

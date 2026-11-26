#!/bin/sh
# Point this container's Debian/Ubuntu apt sources at HTTPS.
#
# Self-hosted runners whose NetworkPolicy allows external egress on 443/TCP
# only hang on the stock `http://deb.debian.org` sources until they time out:
#
#   Err:1 http://deb.debian.org/debian bookworm InRelease
#     Could not connect to debian.map.fastlydns.net:80 (...), connection timed out
#
# Debian and Ubuntu serve the same mirrors over HTTPS, so rewriting the sources
# fixes apt without widening the policy for every workload in the namespace.
#
# Nothing in the Rust workspace needs a system package any more — protoc is
# vendored (crates/sandbox-proto) — but sccache and mold still come from apt,
# and there is no way to install them inside rust:*-bookworm without a working
# package manager. taiki-e/install-action doesn't avoid this: it apt-installs
# its own jq/curl/tar prerequisites and fails here for exactly the same reason.
# The durable fix is baking sccache + mold into the runner image; until then,
# run this before `apt-get update`.
set -e

bootstrap_ca=false
if [ "${1:-}" = "--bootstrap-ca-certificates" ]; then
	bootstrap_ca=true
	shift
fi

if [ "$#" -ne 0 ]; then
	echo "usage: $0 [--bootstrap-ca-certificates]" >&2
	exit 2
fi

# Bookworm images use /etc/apt/sources.list; newer Debian/Ubuntu images use
# deb822 .sources. Both globs can come up empty, hence the per-file guard.
for f in /etc/apt/sources.list /etc/apt/sources.list.d/*.list /etc/apt/sources.list.d/*.sources; do
	[ -f "$f" ] || continue
	sed -i \
		-e 's|http://deb\.debian\.org|https://deb.debian.org|g' \
		-e 's|http://security\.debian\.org|https://security.debian.org|g' \
		-e 's|http://archive\.ubuntu\.com|https://archive.ubuntu.com|g' \
		-e 's|http://security\.ubuntu\.com|https://security.ubuntu.com|g' \
		"$f"
done

if [ "$bootstrap_ca" = true ] && [ ! -s /etc/ssl/certs/ca-certificates.crt ]; then
	apt_no_verify=/etc/apt/apt.conf.d/99bootstrap-ca-certificates
	trap 'rm -f "$apt_no_verify"' EXIT
	{
		echo 'Acquire::https::Verify-Peer "false";'
		echo 'Acquire::https::Verify-Host "false";'
	} > "$apt_no_verify"
	apt-get update
	apt-get install -y --no-install-recommends ca-certificates
	rm -f "$apt_no_verify"
	trap - EXIT
fi

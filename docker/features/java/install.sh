#!/bin/bash
# docker/features/java/install.sh — Eclipse Temurin JDK + Maven.
#
# Runs as root inside a recipe build, after
# `COPY docker/features/java/ /tmp/features/java/` (see docker/generate-recipe.sh).
# The caller sets JAVA_VERSION (feature.json's `version` option, via the
# generator's <FEATURE>_<OPTION> convention) before invoking this script —
# there is no fallback default here on purpose: the recipe pins it, this
# script trusts the recipe.
#
# Adoptium apt repo, not Ubuntu's own openjdk package (noble's repos don't
# reliably carry a JDK 25 build; Adoptium gives 21 and 25 the same way).

set -euo pipefail

: "${JAVA_VERSION:?JAVA_VERSION must be set (e.g. JAVA_VERSION=21)}"

# The base image (sandbox-base or sandbox) already ran
# apt-https-mirrors.sh --bootstrap-ca-certificates once at build time; rerunning
# it here is a no-op on an already-HTTPS sources list and cheap insurance if a
# future base ever stops doing it itself.
apt-https-mirrors.sh --bootstrap-ca-certificates

apt-get update
apt-get install -y --no-install-recommends \
    wget apt-transport-https gpg maven

install -d -m 0755 /etc/apt/keyrings
wget -qO - https://packages.adoptium.net/artifactory/api/gpg/key/public \
    | gpg --dearmor -o /etc/apt/keyrings/adoptium.gpg
echo "deb [signed-by=/etc/apt/keyrings/adoptium.gpg] https://packages.adoptium.net/artifactory/deb $(. /etc/os-release && echo "$VERSION_CODENAME") main" \
    > /etc/apt/sources.list.d/adoptium.list

apt-get update
apt-get install -y --no-install-recommends "temurin-${JAVA_VERSION}-jdk"
rm -rf /var/lib/apt/lists/*

# Stable path independent of the exact installed package-version string.
jdk_dirs=(/usr/lib/jvm/temurin-"${JAVA_VERSION}"-jdk-*)
jdk_home="${jdk_dirs[0]}"
if [ ! -x "${jdk_home}/bin/javac" ]; then
    echo "temurin-${JAVA_VERSION}-jdk installed no JDK under /usr/lib/jvm" >&2
    exit 1
fi

# -n, deliberately: the base image ships default-java as a symlink to its own
# openjdk tree, and without it `ln -sf` follows that link and writes the new
# symlink *inside* the old JDK, leaving default-java pointing at 21.
ln -sfn "${jdk_home}" /usr/lib/jvm/default-java

# The base's openjdk and an Adoptium build of the same major register at the
# same alternatives priority, so auto mode picks between them arbitrarily —
# which is how `javac` came from one JDK and `java` from another.
while read -r _ name path; do
    update-alternatives --set "${name}" "${path}"
done < <(grep -E "^[a-z]+ +[a-z]+ +${jdk_home}/bin/" "/usr/lib/jvm/.$(basename "${jdk_home}").jinfo")

# Every route to a JVM must land on the requested major: JAVA_HOME and PATH
# (feature.json's env) and /usr/bin through alternatives. A split between them
# compiles class files the runtime then rejects with UnsupportedClassVersionError.
for bin in /usr/lib/jvm/default-java/bin/java /usr/lib/jvm/default-java/bin/javac \
           /usr/bin/java /usr/bin/javac; do
    version="$("${bin}" -version 2>&1 | head -1)"
    # `java -version` quotes the version and `javac -version` does not.
    case "${version}" in
        "openjdk version \"${JAVA_VERSION}."*) ;;
        "javac ${JAVA_VERSION}."*) ;;
        *) echo "${bin} reports '${version}', expected Java ${JAVA_VERSION}" >&2; exit 1 ;;
    esac
done

# Maven sources /etc/mavenrc on every run; it mints the CodeArtifact token.
install -m 0644 "$(dirname "$0")/mavenrc" /etc/mavenrc

# Eclipse JDT.LS for the `java` language server, with the Lombok agent that
# lets it see generated builders and accessors. Runs on JDK 21 or later.
# Bump the version, build stamp and checksum together, by hand: the download
# URL needs all three (milestones/<version>/latest.txt names the build).
JDTLS_VERSION=1.62.0
JDTLS_BUILD=202610131109
JDTLS_SHA256=f587dafd8410633c2c2bf005e853a2bbb03cddee88933d49bfe6d9cb1cf09342
LOMBOK_VERSION=1.18.48
LOMBOK_SHA1=6858f13541bab505384f07053c5a7b539bbfd3e3
install -d -m 0755 /opt/jdtls
wget -qO /tmp/jdtls.tar.gz \
    "https://download.eclipse.org/jdtls/milestones/${JDTLS_VERSION}/jdt-language-server-${JDTLS_VERSION}-${JDTLS_BUILD}.tar.gz"
echo "${JDTLS_SHA256}  /tmp/jdtls.tar.gz" | sha256sum -c -
tar -xzf /tmp/jdtls.tar.gz -C /opt/jdtls
rm /tmp/jdtls.tar.gz
wget -qO /opt/jdtls/lombok.jar \
    "https://repo1.maven.org/maven2/org/projectlombok/lombok/${LOMBOK_VERSION}/lombok-${LOMBOK_VERSION}.jar"
echo "${LOMBOK_SHA1}  /opt/jdtls/lombok.jar" | sha1sum -c -
install -m 0755 "$(dirname "$0")/moatless-jdtls" /usr/local/bin/moatless-jdtls

# The real JAVA_HOME/PATH ENV is emitted into the generated Dockerfile from
# this feature's feature.json "env" field (see generate-recipe.sh) — that's
# what the sandbox process itself inherits. This /etc/profile append is only
# a secondary convenience for interactive login shells, same convention
# sandbox-base uses for RUSTUP_HOME/CARGO_HOME/BUN_INSTALL/VP_HOME.
{
  echo 'export JAVA_HOME=/usr/lib/jvm/default-java'
  echo 'export PATH="/usr/lib/jvm/default-java/bin:$PATH"'
} >> /etc/profile

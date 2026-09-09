#!/usr/bin/env bash
# Builds party C's images (Java EDC control plane, data plane and IdentityHub) from the
# MinimumViableDataspace launchers.
#
# MVD's main branch targets a snapshot version of EDC that is no longer published, so the
# checkout is forward-ported to the pinned release with party-c/mvd-edc-1.0.0-RC1.patch
# before building. The patch is idempotent: it is skipped when already applied.
set -euo pipefail

EDC_VERSION="${EDC_VERSION:-1.0.0-RC1}"
MVD_DIR="${MVD_DIR:-$(cd "$(dirname "$0")/../../MinimumViableDataspace" 2>/dev/null && pwd || true)}"
PATCH="$(cd "$(dirname "$0")" && pwd)/party-c/mvd-edc-$EDC_VERSION.patch"

if [[ -z "$MVD_DIR" || ! -f "$MVD_DIR/settings.gradle.kts" ]]; then
  echo "MinimumViableDataspace checkout not found. Clone it next to this repo, or set MVD_DIR." >&2
  echo "  git clone https://github.com/eclipse-edc/MinimumViableDataspace.git" >&2
  exit 1
fi

echo "==> MVD: $MVD_DIR (EDC $EDC_VERSION)"

if grep -q "^edc = \"$EDC_VERSION\"" "$MVD_DIR/gradle/libs.versions.toml"; then
  echo "==> patch already applied"
elif git -C "$MVD_DIR" apply --check "$PATCH" 2>/dev/null; then
  git -C "$MVD_DIR" apply "$PATCH"
  echo "==> applied $PATCH"
else
  echo "$PATCH does not apply to this MVD checkout." >&2
  echo "Expected base commit 17bb5b87cacf66a5db835f718e99db1a97bfe667." >&2
  exit 1
fi

echo "==> building launcher jars"
(cd "$MVD_DIR" && ./gradlew --console=plain \
  :launchers:controlplane:shadowJar \
  :launchers:dataplane:shadowJar \
  :launchers:identity-hub:shadowJar)

build_image() {
  local launcher="$1" jar="$2" image="$3"
  echo "==> docker build $image"
  docker build \
    --build-arg "JAR=build/libs/$jar" \
    -f "$MVD_DIR/launchers/$launcher/src/main/docker/Dockerfile" \
    -t "$image" \
    "$MVD_DIR/launchers/$launcher"
}

build_image controlplane  controlplane.jar  "party-c-controlplane:$EDC_VERSION"
build_image dataplane     dataplane.jar     "party-c-dataplane:$EDC_VERSION"
build_image identity-hub  identity-hub.jar  "party-c-identityhub:$EDC_VERSION"

echo
echo "Built:"
docker images --format '{{.Repository}}:{{.Tag}}\t{{.Size}}' | grep '^party-c-' || true

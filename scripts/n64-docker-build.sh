#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache="${LIBDRAGON_DOCKER_CACHE:-$root/.cache/libdragon}"
source_dir="$cache/source"
image="${LIBDRAGON_DOCKER_IMAGE:-chipviz-libdragon-toolchain:local}"
ref="${LIBDRAGON_BRANCH:-e356bf3f56f7afbf7e5246329562f145965cfdfc}"

mkdir -p "$cache"

if [ ! -d "$source_dir/.git" ]; then
  if [ -e "$source_dir" ] && [ -n "$(ls -A "$source_dir")" ]; then
    echo "Refusing to overwrite non-Git source cache: $source_dir" >&2
    exit 1
  fi
  git init "$source_dir"
  git -C "$source_dir" remote add origin https://github.com/DragonMinded/libdragon.git
fi
if [ -n "$(git -C "$source_dir" status --porcelain --untracked-files=no)" ]; then
  echo "Refusing to build from a modified source cache: $source_dir" >&2
  exit 1
fi
git -C "$source_dir" fetch --depth 1 origin "$ref"
git -C "$source_dir" checkout --detach FETCH_HEAD

source_rev="$(git -C "$source_dir" rev-parse HEAD)"
if [ -z "${LIBDRAGON_DOCKER_IMAGE:-}" ]; then
  docker build --platform linux/amd64 --provenance=false --tag "$image" \
    -f "$root/scripts/n64-toolchain.Dockerfile" "$root/scripts"
elif ! docker image inspect "$image" >/dev/null 2>&1; then
  docker pull --platform linux/amd64 "$image"
fi
image_id="$(docker image inspect --format '{{.Id}}' "$image")"
build_dir="$cache/builds/$source_rev-${image_id#sha256:}"
mkdir -p "$build_dir/sdk"
if [ ! -d "$build_dir/source/.git" ]; then
  git clone --no-hardlinks "$source_dir" "$build_dir/source"
fi
if [ "$(git -C "$build_dir/source" rev-parse HEAD)" != "$source_rev" ] ||
   [ -n "$(git -C "$build_dir/source" status --porcelain --untracked-files=no)" ]; then
  echo "Refusing to build from a modified SDK build cache: $build_dir/source" >&2
  exit 1
fi

docker run --rm --platform linux/amd64 \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e LIBDRAGON_BUILD_ID="$source_rev $image_id" \
  -e N64_INST=/cache/sdk \
  -e N64_GCCPREFIX=/n64_toolchain \
  -v "$root":/workspace \
  -v "$build_dir":/cache \
  -w /workspace/cores/n64/homebrew \
  "$image_id" \
  sh -lc '
    set -e
    jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1)"
    if [ ! -f /cache/sdk/include/n64.mk ] ||
       [ ! -f /cache/sdk/mips64-elf/lib/libdragon.a ] ||
       [ ! -x /cache/sdk/bin/n64tool ] ||
       [ "$(cat /cache/sdk/.libdragon-build 2>/dev/null || true)" != "$LIBDRAGON_BUILD_ID" ]; then
      rm -f /cache/sdk/.libdragon-build
      make -C /cache/source -j"$jobs" install-mk
      make -C /cache/source -j"$jobs" clobber
      make -C /cache/source -j"$jobs" libdragon tools
      make -C /cache/source -j"$jobs" install tools-install
      printf "%s\n" "$LIBDRAGON_BUILD_ID" > /cache/sdk/.libdragon-build
    fi
    if [ "$(cat build/.chipviz-toolchain 2>/dev/null || true)" != "$LIBDRAGON_BUILD_ID" ]; then
      make clean
    fi
    make "$@"
    if [ -d build ]; then
      printf "%s\n" "$LIBDRAGON_BUILD_ID" > build/.chipviz-toolchain
    fi
  ' sh "$@"

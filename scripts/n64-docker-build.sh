#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cache="${LIBDRAGON_DOCKER_CACHE:-$root/.cache/libdragon}"
source_dir="$cache/source"
sdk_dir="$cache/sdk"
image="${LIBDRAGON_DOCKER_IMAGE:-ghcr.io/dragonminded/libdragon:latest}"
branch="${LIBDRAGON_BRANCH:-trunk}"

mkdir -p "$cache" "$sdk_dir"

if [ ! -d "$source_dir/.git" ]; then
  rm -rf "$source_dir"
  git clone --depth 1 --branch "$branch" https://github.com/DragonMinded/libdragon.git "$source_dir"
else
  git -C "$source_dir" fetch --depth 1 origin "$branch"
  git -C "$source_dir" checkout --detach FETCH_HEAD
fi

source_rev="$(git -C "$source_dir" rev-parse HEAD)"

docker run --rm --platform linux/amd64 \
  --user "$(id -u):$(id -g)" \
  -e HOME=/tmp \
  -e LIBDRAGON_SOURCE_REV="$source_rev" \
  -e N64_INST=/cache/sdk \
  -e N64_GCCPREFIX=/n64_toolchain \
  -v "$root":/workspace \
  -v "$cache":/cache \
  -w /workspace/cores/n64/homebrew \
  "$image" \
  sh -lc '
    set -e
    jobs="$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 1)"
    if [ ! -f /cache/sdk/include/n64.mk ] ||
       [ ! -f /cache/sdk/mips64-elf/lib/libdragon.a ] ||
       [ ! -x /cache/sdk/bin/n64tool ] ||
       [ "$(cat /cache/sdk/.libdragon-rev 2>/dev/null || true)" != "$LIBDRAGON_SOURCE_REV" ]; then
      make -C /cache/source -j"$jobs" install-mk
      make -C /cache/source -j"$jobs" clobber
      make -C /cache/source -j"$jobs" libdragon tools
      make -C /cache/source -j"$jobs" install tools-install
      printf "%s\n" "$LIBDRAGON_SOURCE_REV" > /cache/sdk/.libdragon-rev
    fi
    make "$@"
  ' sh "$@"

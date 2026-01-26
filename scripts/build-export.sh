#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BUILD_FILE="${ROOT_DIR}/scripts/build-number.txt"
OUT_DIR="${1:-${ROOT_DIR}/firmware}"
PROFILE="${2:-release}"

if [[ "$PROFILE" != "release" && "$PROFILE" != "debug" ]]; then
  echo "Profile must be 'release' or 'debug'." >&2
  exit 1
fi

current="0"
if [[ -f "$BUILD_FILE" ]]; then
  current="$(tr -d '[:space:]' < "$BUILD_FILE")"
  if [[ -z "$current" ]]; then
    current="0"
  fi
fi

if [[ ! "$current" =~ ^[0-9]+$ ]]; then
  echo "Invalid build number in ${BUILD_FILE}: ${current}" >&2
  exit 1
fi

next=$((10#$current + 1))
build_str="$(printf "%04d" "$next")"

echo "$build_str" > "$BUILD_FILE"

echo "Building OTA (${PROFILE}): ${build_str}"
if [[ "$PROFILE" == "release" ]]; then
  OTA_BUILD="$build_str" "${ROOT_DIR}/scripts/build-release.sh"
else
  OTA_BUILD="$build_str" cargo build
fi
${ROOT_DIR}/scripts/export-ota.sh "$build_str" "$OUT_DIR" "$PROFILE"

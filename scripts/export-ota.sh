#!/usr/bin/env bash
set -euo pipefail

usage() {
  echo "Usage: $0 <build-number> [output-dir] [profile]" >&2
  echo "  profile: release (default) or debug" >&2
  echo "Example: $0 0001 ./firmware release" >&2
  echo "Example: $0 0001 ./firmware debug" >&2
}

if [[ $# -lt 1 || $# -gt 3 ]]; then
  usage
  exit 1
fi

build="$1"
if [[ ! "$build" =~ ^[0-9]+$ ]]; then
  echo "Build number must be digits only." >&2
  exit 1
fi

shift
out_dir="$PWD"
profile="release"

if [[ $# -ge 1 ]]; then
  case "$1" in
    release|debug)
      profile="$1"
      shift
      ;;
    *)
      out_dir="$1"
      shift
      ;;
  esac
fi

if [[ $# -ge 1 ]]; then
  case "$1" in
    release|debug)
      profile="$1"
      shift
      ;;
    *)
      echo "Unknown argument: $1" >&2
      usage
      exit 1
      ;;
  esac
fi

if [[ $# -gt 0 ]]; then
  usage
  exit 1
fi

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
elf="${root_dir}/target/riscv32imac-esp-espidf/${profile}/c6-demo"

if [[ ! -f "$elf" ]]; then
  if [[ "$profile" == "debug" ]]; then
    echo "No debug ELF found. Run cargo build first." >&2
  else
    echo "No release ELF found. Run scripts/build-release.sh first." >&2
  fi
  exit 1
fi

if ! command -v espflash >/dev/null 2>&1; then
  echo "espflash not found in PATH. Install it or run via cargo +nightly espflash." >&2
  exit 1
fi
mkdir -p "$out_dir"

dest="${out_dir}/c6-co${build}.bin"
espflash save-image \
  --chip esp32c6 \
  --flash-size 8mb \
  --flash-mode dio \
  --flash-freq 80mhz \
  "$elf" \
  "$dest"
echo "c6-co${build}.bin" > "${out_dir}/latest.txt"

echo "Wrote: ${dest}"
echo "Wrote: ${out_dir}/latest.txt"

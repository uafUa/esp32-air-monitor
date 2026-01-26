#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
ESP_DIR="${ROOT_DIR}/.embuild/espressif"
IDF_VERSION="v5.3.3"
RISC_V_VERSION="esp-13.2.0_20240530"

export PATH="${ESP_DIR}/python_env/idf5.3_py3.12_env/bin:${ESP_DIR}/esp-idf/${IDF_VERSION}/tools:${ESP_DIR}/tools/riscv32-esp-elf/${RISC_V_VERSION}/riscv32-esp-elf/bin:${PATH}"

exec cargo +nightly build --release

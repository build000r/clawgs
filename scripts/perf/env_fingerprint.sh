#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(git -C "$script_dir/../.." rev-parse --show-toplevel 2>/dev/null || (cd "$script_dir/../.." && pwd))"
out_path="$repo_root/tests/artifacts/perf/fingerprint.json"

json_escape() {
  local value="${1:-}"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  value="${value//$'\n'/\\n}"
  value="${value//$'\r'/\\r}"
  value="${value//$'\t'/\\t}"
  printf '%s' "$value"
}

platform="$(uname -s 2>/dev/null || printf 'unknown')"
kernel_release="$(uname -r 2>/dev/null || printf 'unknown')"
machine="$(uname -m 2>/dev/null || printf 'unknown')"

cpu_model="unknown"
cpu_cores="unknown"
ram_total="unknown"
os_name="$platform"
os_version="unknown"
os_build="unknown"

if [[ "$platform" == "Darwin" ]]; then
  cpu_model="$(sysctl -n machdep.cpu.brand_string 2>/dev/null || printf 'unknown')"
  cpu_cores="$(sysctl -n hw.ncpu 2>/dev/null || printf 'unknown')"
  ram_total="$(sysctl -n hw.memsize 2>/dev/null || printf 'unknown')"
  os_name="$(sw_vers -productName 2>/dev/null || printf 'macOS')"
  os_version="$(sw_vers -productVersion 2>/dev/null || printf 'unknown')"
  os_build="$(sw_vers -buildVersion 2>/dev/null || printf 'unknown')"
elif [[ "$platform" == "Linux" ]]; then
  cpu_model="$(awk -F': ' '/model name/{print $2; exit}' /proc/cpuinfo 2>/dev/null || true)"
  if [[ -z "$cpu_model" ]]; then
    cpu_model="$machine"
  fi
  cpu_cores="$(getconf _NPROCESSORS_ONLN 2>/dev/null || nproc 2>/dev/null || printf 'unknown')"
  ram_total="$(awk '/MemTotal/{print $2 " kB"; exit}' /proc/meminfo 2>/dev/null || true)"
  if [[ -z "$ram_total" ]]; then
    ram_total="unknown"
  fi
  os_name="$(awk -F= '/^NAME=/{gsub(/^"|"$/, "", $2); print $2; exit}' /etc/os-release 2>/dev/null || printf 'Linux')"
  os_version="$(awk -F= '/^VERSION=/{gsub(/^"|"$/, "", $2); print $2; exit}' /etc/os-release 2>/dev/null || printf 'unknown')"
  os_build="$(awk -F= '/^VERSION_ID=/{gsub(/^"|"$/, "", $2); print $2; exit}' /etc/os-release 2>/dev/null || printf 'unknown')"
fi

timestamp="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"
hostname_value="$(hostname 2>/dev/null || printf 'unknown')"
rustc_version="$(rustc --version 2>/dev/null || printf 'unavailable')"
cargo_version="$(cargo --version 2>/dev/null || printf 'unavailable')"
git_head="$(git -C "$repo_root" rev-parse HEAD 2>/dev/null || printf 'unknown')"
git_status_short="$(git -C "$repo_root" status --short 2>/dev/null || true)"
profile="release-perf"

json="$(printf '{
  "timestamp": "%s",
  "hostname": "%s",
  "profile": "%s",
  "cpu": {
    "model": "%s",
    "core_count": "%s"
  },
  "ram": {
    "total": "%s"
  },
  "os": {
    "name": "%s",
    "version": "%s",
    "build": "%s"
  },
  "kernel": {
    "name": "%s",
    "release": "%s",
    "machine": "%s"
  },
  "rustc": "%s",
  "cargo": "%s",
  "git": {
    "head": "%s",
    "status_short": "%s"
  }
}
' \
  "$(json_escape "$timestamp")" \
  "$(json_escape "$hostname_value")" \
  "$(json_escape "$profile")" \
  "$(json_escape "$cpu_model")" \
  "$(json_escape "$cpu_cores")" \
  "$(json_escape "$ram_total")" \
  "$(json_escape "$os_name")" \
  "$(json_escape "$os_version")" \
  "$(json_escape "$os_build")" \
  "$(json_escape "$platform")" \
  "$(json_escape "$kernel_release")" \
  "$(json_escape "$machine")" \
  "$(json_escape "$rustc_version")" \
  "$(json_escape "$cargo_version")" \
  "$(json_escape "$git_head")" \
  "$(json_escape "$git_status_short")")"

mkdir -p "$(dirname "$out_path")"
printf '%s' "$json" | tee "$out_path"

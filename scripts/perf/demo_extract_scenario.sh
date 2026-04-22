#!/usr/bin/env bash
set -euo pipefail

# Scenario D: demo extract corpus profile
# Each invocation is ~5-10ms, so we loop 50 iterations of both tool variants
# to accumulate enough work for meaningful measurement.

CLAWGS="${CLAWGS_BIN:-target/release-perf/clawgs}"
N=50

for (( i=0; i<N; i++ )); do
  "$CLAWGS" demo extract --tool codex --pretty >/dev/null
  "$CLAWGS" demo extract --tool claude --pretty >/dev/null
done

printf 'scenario D run complete\n'

| Command | Mean [ms] | Min [ms] | Max [ms] | Relative |
|:---|---:|---:|---:|---:|
| `jq -c '.config.enabled=false' scripts/perf/fixtures/emit_stdio.ndjson \| OPENROUTER_API_KEY=fake target/release-perf/clawgs emit --stdio >/dev/null` | 5.9 ± 0.7 | 4.9 | 7.8 | 1.00 |

# OpenWeights 0.17.0: validation record

## Frontend baseline

Measured on the same Windows machine using Edge, Vite development mode and
React Profiler on 2026-09-07. Baseline: v0.16.1 (`e598a5c`). The fixture is
`tests/render.tsx`: 500 history messages, a growing TypeScript code block,
500 fragments delivered at 100 fragments/s, three repetitions. The current
fixture uses the virtualized MessageList; batch size 5 models 50 ms publication.
The unit test verifies the actual store's publication limit separately.

| Version | Render CPU samples (ms) | Median (ms) | DOM nodes |
| --- | --- | --- | --- |
| 0.16.1, batch 1 | 2757.3, 2576.1, 2688.6 | 2688.6 | 4278 |
| 0.17.0, batch 5 | 199.0, 170.7, 147.1 | 170.7 | 161 |

The median reduction is 93.7%. This is render CPU in a synthetic development
fixture, not total application CPU, a release WebView profile, or model tokens/s.
Reproduce with `npm run dev` and `npm run bench:render -- result.json 5`.
For the old baseline, use the v0.16.1 MessageList dependencies and batch 1.

## Real inference

Windows, llama.cpp b10441 CUDA 13.3, Qwen3.8-27B-GSQ-RCO-IQ3_S, context 8192,
128 generated tokens. `scripts/inference-bench.mjs` compares the frozen
v0.16.1 SSE client with the new client on the same running server, alternating
order. Set `OW_TEST_RUNTIME` and `OW_TEST_MODEL`, then run with Vite available.

The initial samples had large outliers (responses between 4 and 29 seconds).
They do not establish the controlled ≤5% non-regression criterion. The isolated
comparison integration test successfully warmed up, collected three samples,
read timings and stopped its owned server, but its samples were also noisy.
No engine speedup is claimed from those measurements.

A later repeat, after warmup of both clients, expanded to nine alternating
samples per client (`node scripts/inference-bench.mjs inference-bench-expanded.json 9`):

| Metric | 0.16.1 median (min–max) | 0.17.0 median (min–max) |
| --- | --- | --- |
| First token, ms | 99.8 (97.2–109.5) | 101.7 (98.8–111.2) |
| Response, ms | 2967.2 (2941.8–3110.2) | 2966.1 (2924.5–3086.5) |
| Generation, tokens/s | 44.314 (42.228–44.650) | 44.341 (42.685–44.991) |

First-token median changed +1.9%; total latency −0.04%; generation +0.06%.
These medians meet the 5% non-regression threshold in this Windows workload.
First-token jitter remains about 12 ms, so this is not evidence of a speedup
and does not establish performance on other hardware or native WebViews.

The comparator rejects noisy recommendations and preserves context, precision,
reasoning settings and custom extras. Available memory comes from the engine's
device listing and is not a peak-allocation measurement. Slot checks fail
closed before interrupting the local server; other GPU applications and the
small interval between checking activity and stopping the server cannot be
fully observed. Avoid external clients during a comparison.

## Automated coverage

- Frontend: exact UTF-8/SSE text, partial failure/cancellation, missing metrics,
  20 Hz publication, per-conversation subscriptions, local/remote scheduling,
  bounded retries, title priority/input, benchmark lease and navigation state.
- Browser: generation/cancellation, virtualized 500-message history, stable
  reading position and typing during streaming, performance entry point.
- Rust: additive metrics persistence and legacy records, comparison history,
  invariant settings, sample validation and fail-closed slot checks, plus the
  existing workspace tests.
- CI builds/checks/tests the Rust workspace on Windows, macOS and Linux; real
  inference measurements above were performed only on Windows.

Real GPU integration is opt-in:
`cargo test -p openweights real_isolated_arm_warms_up_measures_and_stops -- --ignored --nocapture`.
Do not run it alongside active inference or GPU-heavy applications.

# Chat performance

Responses now distinguish queueing, server preparation, confirmed model loading,
waiting for the first token and generation. **Run details** separates the first
token (which may be reasoning) from the first visible answer. Wait and total
durations are measured by the app; token counts, prompt/cache data and generation
speed come from the engine. Missing engine data stays **Not reported**.

Long conversations use a virtualized history. Streaming text is published at
50 ms intervals, with immediate completion, error and cancellation updates.
Code highlighting runs when the response finishes. Cancelling preserves the
partial response, including reasoning and recorded metrics.

## Compare configurations

Open **Local Server → Performance**, select a model and choose **Measure and
compare**. Set an explicit context size first. The advisor must provide a
different execution configuration: model, quantization, context, reasoning
settings and custom extras remain unchanged.

The test requires an idle server and pauses new internal work. It temporarily
stops the local API server and measures on an isolated loopback server. Avoid
external clients and GPU-heavy programs during the test. If slot activity
cannot be verified, the test will not start.

Each configuration gets one warmup and three repetitions of the same versioned
code prompt. Results show medians and minimum–maximum ranges. Available memory
is the sum reported by the engine's device listing after warmup, not peak
memory usage or memory allocated by the model. Drivers and shared memory can
affect that reading; missing data is left unavailable.

A candidate is only recommended when generation improves beyond the observed
ranges, the ranges remain within 10% of their medians, and prompt speed and total
latency do not regress by more than 5%. Noisy results are inconclusive: repeat
with other GPU work stopped. These checks detect variation, not every possible
external source of interference.

Choose **Apply candidate** or **Keep current**. After applying, **Restore previous
configuration** brings back the saved profile. Measurement never saves a trial
profile, and cancellation or failure restores server availability when it was
running. You can leave the screen and return to cancel an ongoing test.

## Measurement scope

The Windows development benchmark with 500 history messages and 100 synthetic
fragments/s reduced median React render CPU from 2,688.6 ms to 170.7 ms in three
runs (93.7%). This measures the frontend fixture, not an increase in model
tokens/s. Real inference must be measured separately on the same hardware,
runtime build, model and configuration. See the repository's
[validation record](https://github.com/pedro-canedo/openweights/blob/main/docs/performance-0.17.0.md)
for methodology and limitations.

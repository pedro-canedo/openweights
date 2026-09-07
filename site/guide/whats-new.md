# What's new

This page covers what changed in the app you would download today, and why.
The full history of every version is in the [changelog](/guide/changelog);
installers live on
[GitHub](https://github.com/pedro-canedo/openweights/releases).

## 0.17.0 — smoother chat and comparable configurations

Long conversations are virtualized, streaming updates are batched at 50 ms,
and code highlighting waits until the answer finishes. Cancelling keeps the
partial response. Background generations no longer refresh every conversation.

Explicit waiting phases and persistent run metrics distinguish queue time,
model loading, reasoning and visible answers. Local work respects the configured
capacity; automatic titles have lower priority and use bounded input.

**Local Server → Performance** can now compare the current configuration with
an advisor candidate using warmup and three repetitions per configuration.
Results include ranges, an inconclusive verdict for noisy measurements, and
explicit apply/restore actions. See [Chat performance](./performance) for the
workflow and measurement limits. This release does not claim faster model inference.

## 0.16.1 — the engine check read the wrong number

Fixes a 0.16.0 bug that showed up for everyone: the engine card said **"the
engine is installed but won't run"** even while llama.cpp was serving a model.

The output of `llama-server --version` is
`version: 0.1.0-dev (build 10441, commit 0177dcc73)`, and the check read the
first number after `version:` — the zero in `0.1.0-dev`. Since the folder is
named `b10441` and the binary "answered" build 0, the verdict was a mismatch
between folder and contents. The number now comes from `build`, which is where
it always was.

## 0.16.0 — the screens gained shape, and the engine gets checked

### The engine is verified, not assumed

The app installs its own llama.cpp, in its own folder, isolated from anything
you may have on your system. Until now the Settings card read
`b10441 · cuda13 · installed` — a version number that came from the build the
app *would* install, plus the presence of a file on disk. Both halves could be
wrong at the same time: someone who updated OpenWeights read the new number
while still running the old build, and a CUDA package missing its runtime DLLs
passes a file check and only fails when you load a model.

The app now **runs** `llama-server --version` and reads the build the binary
itself reports. The answer is one of five, each with its own fix:

| Verdict | What it means |
|---|---|
| **Ready** | It ran and answered with the build this release expects |
| **Not installed** | First run, or the folder was deleted |
| **Update available** | The disk has another build than this release was tested with |
| **Wrong variant** | Your GPU or driver changed since the install |
| **Won't run** | The files are there and the executable does not start |

The check runs on its own shortly after startup, and whatever needs attention
shows as a dot on the **Settings** item in the sidebar — you find out before a
conversation fails, not during one. Builds left behind by past updates are
listed with their size and a button that reclaims the space; the one in use is
never touched.

### Local Server: a pinned status bar and four tabs

The screen used to be eleven cards of identical weight. Now the server state —
running or stopped, the address with a copy button, the loaded model and the
generation speed — stays pinned at the top while you scroll, and the rest is
organised by the question you arrive with:

| Tab | What's in it |
|---|---|
| **Overview** | Connect, use it in another app, and what has been served |
| **Performance** | Configure llama.cpp, speculation, benchmark history, GPU power |
| **Network** | Port, local-network access, concurrency, extra GPU on the network |
| **Advanced** | Global flags and the server log |

On a fresh install, Overview opens with three numbered steps — start it, copy
the address, paste it into the app that will use it. They disappear on their
own once the server answers its first request.

### The harness tells you when it is out of date

The version shown is now the one actually installed, read from the package's
own manifest. Every time you open the screen the app compares it with the
version this release ships; a mismatch becomes **update pending**, with the
button that resolves it. The latest version published on npm appears as
information — the app installs the one that passed our tests.

### Model cards render as text again

Hugging Face READMEs are Markdown mixed with HTML, and the app used to show
that HTML literally: a wall of `<div style="display: flex">` where the model
description should be. It is now translated to plain Markdown — links, images,
tables and code blocks included — before it reaches the screen.

### Settings, Sources, and the brand

Settings is ordered by what matters (engine, credential, preferences, machine),
and the Hugging Face token now says whether one is saved — a filled password
field looks exactly like an empty one. Sources replaced the list of status dots
with one card per source, and clicking a card takes you to the tab where that
gets resolved; the OpenRouter catalogue says how many models you have pinned
(only pinned ones reach the chat picker) and warns when the list was cut short.

The mark inside the app is now the same geometry as the icons, generated from a
single source instead of being redrawn by hand in two places.

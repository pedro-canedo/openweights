# What's new

## 0.22.0

**Bonsai models run.** PrismML's Ternary Bonsai 2 files (`PTQ1_0`, `PQ2_0`)
use tensor types the standard llama.cpp refuses, and until now they ended in
an HTTP 500. The app now reads the tensor types from the file header, knows
which files need PrismML's fork of llama.cpp, and **installs that engine
together with the download** — one click, both progress bars in the downloads
panel. Picking a Bonsai model restarts the local server on the PrismML engine;
picking any other model restarts it on the official build. The Local Server
header says which one is running. A Bonsai model without the engine shows an
**Install PrismML engine** button in the chat and in the library instead of an
error. See [models](/guide/models#bonsai-models-and-the-prismml-engine).

Deleting a model while the server is running no longer leaves a ghost entry
that the chat could still pick (and that answered HTTP 500): the model is
unloaded at once and the server restarts when idle.

## 0.21.0

**Jev** joins **Sources → OpenRouter** as a decision layer for local models. A
TypeSafe decision model, reached through the OpenRouter key, looks at each
message and decides whether the local model should think and how hard — none,
medium or high — instead of the conversation's fixed effort. Simple messages
answer faster; hard ones keep the full budget. The answer's run details show
what was decided.

The same decision reaches the coding agents: with the switch on, a small local
proxy on port 11712 sits in front of the engine, and the DeepSeek Harness and
the other agents the app launches are pointed at it. Streaming and every other
request pass through untouched; whenever Jev cannot decide, the request goes
as the agent sent it. Off by default, and the card says what leaves your
machine when it is on. See [external model sources](/integrations/providers#jev-a-decision-layer).

## 0.20.5

9router can be updated directly in **Sources → 9router**. The screen checks
the version available on npm, shows the installed version and offers **Update**
when a newer version is available. Accounts and settings are preserved; a
running service restarts after the replacement and may interrupt requests
in progress.

The new package is prepared before replacing the installation. Download or
preparation failures leave the previous package intact, and npm progress appears
on screen. The **Local machine (llama.cpp)** tab brings together the engine
controls, including checks and updates. OpenRouter receives updates as an
online service.

## 0.20.4

The Studio home screen now has project cards and a history that distinguishes
preparation, training and comparison. Search by name, model or stage and filter
ready models to open them directly in chat. Dates and identifiers help tell
runs of the same document apart.

The module-management and MVP-import section has been removed from this screen.
Navigation saves your project before starting another, and the layout supports
narrow windows in light and dark themes. See the [Studio guide](/guide/studio).

## 0.20.3

Training Studio now has persistent projects, so sources, prepared datasets,
runs and model lineage stay together. The base-model picker includes Qwen3
0.6B and Qwen3 1.7B, with resource estimates and safe import validation for
Hugging Face repositories or local safetensors folders. Quick and recommended
recipes, an Advanced panel, preflight checks and base-versus-trained comparison
make the path from a document to a chat model easier to inspect.

The packaged desktop also starts the Python Studio backend and its UI from the
private runtime included by Tauri. The [Training Studio guide](/guide/studio)
explains the new project and model workflow.

## 0.20.2

Fixes the actual Studio download failure on Windows: the file was opened in
append-only mode, which prevented resizing it before receiving data. Version
0.20.1 did not resolve this issue. Update and retry installation; existing
downloads can resume without deleting your data.

## 0.20.1

The Training Studio installer now handles an existing Windows runtime marker
without failing with `Access denied (os error 5)`. It retries short antivirus
locks and provides a concrete recovery message. The [Training Studio guide](/guide/studio)
now covers preparation, OCR, resume, resource checks, and troubleshooting.

This page covers what changed in the app you would download today, and why.
The full history of every version is in the [changelog](/guide/changelog);
installers live on
[GitHub](https://github.com/pedro-canedo/openweights/releases).

## 0.20.0 — Studio through the updater

Version 0.20.0 removes the desktop prerelease suffix and provides rebuilt,
signed update artifacts. It includes the optional training Studio and local OCR
described below, using the same verified runtime packages.

## 0.20.0-rc.1 — optional local training Studio

The release candidate adds **Train**, with a private Windows runtime, automatic
data preparation, optional local OCR and Q4_K_M export into the model library.
The first recipe uses Qwen3-0.6B and was exercised on an RTX 3090. A short run is
an experiment, not a guarantee of accurate answers. This candidate is separate
from the stable updater channel; training initially targets Windows with NVIDIA.

## 0.19.3 — a clearer workspace

Settings, model sources, the DeepSeek Harness and My Models now share a clearer
visual hierarchy. Management screens explain their purpose, put the main action
where you expect it and keep technical details available without competing with
the next decision.

My Models adds search, library totals and more room for model names. Source
cards explain local llama.cpp, 9router and OpenRouter and open the right
controls. The light theme, narrow layouts, keyboard focus and both translations
were reviewed together.

## 0.19.2 — what the new choice took with it

Letting you pick which measured configuration to use meant loosening the guards
around applying one. One of those guards was doing two jobs, and only one of
them was in the way.

The old check refused to apply unless the profile in use was exactly the one
from before the measurement. That blocked switching between measured options —
the whole point of the new screen — so it went. But it also blocked applying on
top of an adjustment someone made by hand *after* measuring, and that half had
no replacement: change a flag, click "use configuration", and the adjustment was
gone without a word. The two cases are now told apart, and a hand-edited profile
is flagged before the click rather than refused after it.

Two more things came out of the same review. A profile that says nothing about
which engine to use was being read as "use the official one" — and the tuning
advisor never fills that field, so accepting a flag recommendation quietly moved
anyone on the optional engine back to the official one. And a failure while
reverting the engine could abort before the profile was restored, leaving the
new configuration saved with the engine stopped.

All three passed the test suite, the linter and the formatter. None of them was
visible without reading what the change removed alongside what it meant to
remove.

## 0.19.1 — the benchmark was timing its own repetition

The same model, the same profile and the same machine reported 129 tok/s in
the optimization screen, 28 in the performance history and 31 in an actual
conversation. The optimization was the one that was wrong.

Its long prompt was the short one repeated thirty-two times. That looked
harmless and was not: a profile with n-gram speculation lets the draft find the
continuation *inside the prompt itself*, acceptance goes to the ceiling, and
generated-tokens-over-generation-time stops describing the model and starts
describing how repetitive the test text is. The arithmetic matches the symptom
exactly — a draft of up to 4 tokens, and 32 × 4 = 128.

The performance history never fell for it, because `llama-bench` does not use
speculation at all; neither did chat, which reads real text. That is why those
two agreed on about 30 and only the optimization stood apart.

The long prompt now carries four different function shapes and four different
tasks. Varying only the identifier would not have been enough: a single body
under different names still repeats `for value in values { if *value > limit {`
in every block, and a sequence like that is precisely what an n-gram copies.

## 0.19.0 — choose the configuration you measured

- **Choose any completed optimization option.** Each configuration shows
  generation speed, prompt reading speed and total response time. Recommendations
  still consider total time and stability, but you can now select an alternative
  with faster generation and click **Use configuration**, even without a
  recommended winner. Measurement preserves your current profile until that click.
- **History works while the server is running and idle.** Complete saved
  profiles have an explicit action to apply them and load the model. Older
  measurements without a complete profile are identified; the displayed power
  limit belongs to the measurement and is not changed when reapplying a profile.
- **The performance tab has been reorganized.** Cards with icons, separate
  metrics, an active configuration indicator and a responsive layout highlight
  optimization and history. Manual settings and presets live in an expandable section.
- **Applying keeps controls in sync.** The editor and history refresh after a
  successful application. The choice uses the engine associated with the profile;
  active work prevents application, and loading failures trigger recovery of the
  previous configuration.

Displayed speeds describe the measured workload, not every conversation. This
version does not automatically change settings when optimization finishes:
select an option and confirm with **Use configuration**.

## 0.18.4 — the download was freezing the whole machine

Parallel downloading, added in 0.18.2, made transfers much faster and then
started freezing everything: the app stopped responding, the pause button did
nothing, and other applications froze along with it. Task Manager showed the
whole bug on one line — **85 MB/s of disk with 0 Mbps of network**. The app was
writing 85 megabytes a second without downloading anything.

On NTFS, setting a file's length moves its end but not its *valid data length*.
Writing into the middle afterwards makes the system physically zero everything
in between first, so the new file cannot expose whatever was in those sectors.
While downloads were sequential this never showed: the file grew byte by byte
and the valid data length went with it. Pre-allocating and then writing from
eight connections at once inverted that — each connection starts far into the
file, and each one triggers a multi-gigabyte zeroing. The disk belongs to the
whole system, so the whole system waited.

Marking the file sparse tells NTFS the untouched regions are holes, and a hole
needs no zeroing. Measured here on a 16 GiB file, writing 1 MiB at the 14 GB
mark: 9.54 s before, 0.51 ms after.

The status bar also stopped breaking. Written labels cost more width than the
numbers they introduced, so on smaller screens "26 GB / 64 GB" split across two
lines and the bar grew. Each meter now carries a pictogram and puts its name on
hover.

Two smaller things in the same release. The performance history table always
filtered by model, but never said which one — and a generation figure means
nothing without the model that produced it, which is how 32 tokens/s in one
table and 143 in another screen could look like a contradiction. Each row also
became a button that returns to that configuration, for the times a change
turns out worse. And an optimization that ends without a winner stops offering
"restore": nothing was applied, so there is nothing to undo.

## 0.18.3 — a public catalogue was hiding behind an expired session

A Hugging Face login lasts a few hours. **Discover**, the model README and the
quantization list each took the client that was built when the window opened,
token and all, and never refreshed it. Once that token expired the Hub answered
401 and the whole screen collapsed into "Something went wrong" — even though
the model catalogue is public and needs no token at all. Refreshing the session
before asking fixes it; on top of that, search now retries anonymously if the
token is refused, so a revoked session cannot hide a public list either.

The error card was part of the problem. It said "Something went wrong" and
dropped the reason into the browser console, where nobody looks. It now carries
a technical details section with the actual message — which is how this bug was
found in the first place, by reading a 401 the app had been swallowing.

## 0.18.2 — a model that was there all along, and a download that uses the line

Large Hugging Face repositories keep each quantization in its own folder —
`UD-Q2_K_XL/`, `UD-Q4_K_XL/`. The downloader preserved that path; the library
scan stopped one level short, at `<author>/<repo>`. So a 73 GB model could
finish downloading, sit on disk with every shard at the exact size the Hub
publishes, and never appear in **My Models**. Nothing said it was there.
Deleting and downloading again would have produced the same silence. The scan
now goes into those folders, and models already on disk show up on the next
launch without re-downloading anything.

Downloads also stopped using a single pipe. A 73 GB transfer was moving at
8 MB/s on a 206 Mbps line, and it was not the Hugging Face end: the CDN hands
over 20 MB/s when you ask in parallel. One TCP connection carries at most
`window ÷ round-trip time`, and the round trip to the LFS CDN is 146 ms. The
app opened one connection per file, and downloaded files one after another.

Now the shards of a model download together, and each large file is split into
ranges with a connection of its own. That last part came with a catch worth
knowing: the CDN speaks HTTP/2, and over HTTP/2 an HTTP client multiplexes
concurrent requests to the same host onto a *single* TCP connection — so eight
ranges went right back to sharing one window, and the parallelism was
decorative. Asking for HTTP/1.1 on the byte transfers is what made it real.

Measured end to end against the Hub, the same 742 MiB file went from 41.9 s to
21.9 s, peaking at 51.5 MB/s, and the reassembled file's SHA256 matched the
published one exactly. Files under 16 MB keep a single connection, because a
small file finishes before a fresh connection stops accelerating.

On Windows, the desktop shortcut is back. It had been created only on a fresh
install: the Tauri installer's shortcut routine bails out when it is running as
an update, and the app's updater always runs it that way. So anyone who
installed once and has updated ever since never got a shortcut, no matter how
many versions went by — the app was there, and finding it meant searching the
Start menu. The rule itself is right, since a shortcut someone deleted on
purpose should not come back behind their back; it just could not tell "deleted
it" from "never had one". Now a mark in the registry tells them apart.

## 0.18.0 — one button decides the engine, the threads and the expert cache

Tuning a mixture-of-experts model used to mean knowing that a llama.cpp fork
with a GPU expert cache exists, building it, guessing how many cache slots fit
on the card, and comparing it against the official engine by hand. Almost
nobody does that — and whoever does it once never redoes it after switching
models.

**Optimize for my computer** runs the whole cycle. It downloads the optional
engine at a pinned revision, checks its identity and SHA256 against the
release, runs the binary to prove it actually exposes the capabilities it
claims, and only then measures: your current profile, two thread counts, and —
when the GGUF declares routed experts — the fork without cache and with 16, 32
and 64 slots. Warmup discarded, three repetitions, a short prompt and a long
one, median with range, observed RAM and VRAM.

The official engine stays the default, and the optional one is only considered
when the model file itself says it has routed experts. Cache sizing reads the
file's own geometry rather than a bytes-per-parameter table that would age with
every new quantization; without that geometry the fork arm does not run, and
says why. If the package has no published digest, the app refuses to execute an
unverified binary and stays on the official engine.

Nothing is applied on its own. Your manual profile is preserved, applying
stores exactly what was measured, and hardware, engine or model file changing
after the fact invalidates the evidence instead of silently applying it.

Gated models also stopped asking for a pasted token: signing in to Hugging Face
happens in the browser. Users who had already accepted a licence were seeing
the "gate closed" warning forever, because it came from a repository field that
never changes; the verdict now comes from asking the gate itself. And a missing
token, a revoked token and an unaccepted licence — three different problems —
stopped sharing one sentence.

Icons stopped being text characters. Thirty glyphs — `✓ × ▾ ▸ ♥ ↓ ↑ ↗ ⚠ ⧉ → •
⭐` — were standing in for icons across sixteen files, which meant the system's
installed fonts, not the app, decided their shape and weight. They are now line
pictograms drawn on one grid with one stroke weight. Where the drawing carries
the meaning on its own — the arrow that tells a download rate from an upload
one, the heart that says a number is likes — it is announced to screen readers
instead of hidden.

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

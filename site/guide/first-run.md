# First run

The first launch does three things you never have to repeat: it looks at your
machine, downloads the engine that matches it, and offers you a first model.

## 1. Hardware detection

OpenWeights reads CPU, RAM, GPU and VRAM. That is what every later
recommendation is built on — how much of a model fits on the card, how many
layers to offload, which quantization gets a green light.

The numbers stay visible in the status bar at the bottom of the window: CPU,
RAM, GPU, VRAM, disk and network, live, next to the tokens/s of whatever is
generating.

## 2. The AI engine

The app then downloads the llama.cpp build for your hardware — **a few hundred
MB**, once:

| Your card | What gets downloaded |
|---|---|
| NVIDIA | CUDA build (plus the CUDA runtime redistributed by llama.cpp) |
| AMD, Intel, Apple and others | Vulkan or Metal build |
| No usable GPU | CPU-only build |

This is why the installer is small: no GPU stack ships inside the package. The
CUDA runtime comes straight from the llama.cpp release, on your machine, subject
to the [NVIDIA CUDA EULA](https://docs.nvidia.com/cuda/eula/) — the OpenWeights
installer does not redistribute NVIDIA libraries.

That engine belongs to **the app**: it lives in an app folder, isolated from any
llama.cpp you may already have on your system. Shortly after startup the app
checks it on its own — it runs the binary and reads the build it reports, which
is what separates "the files are there" from "the engine works". If something
needs attention (an old build after an OpenWeights update, the wrong variant
because your GPU or driver changed, an incomplete package), a dot shows up on
the **Settings** item in the sidebar, and the engine card there says what to do
in one sentence. Old builds left behind are listed with their size and a button
to reclaim the space.

## 3. Your first model

Once the engine is ready, the welcome window suggests **three models that fit
this computer**, each in the version that runs best here (on a machine without
a usable GPU, a light 4-bit version, because on the CPU size is speed). **Download
and chat** fetches it and, when the download finishes, opens Chat with that
model already selected. Closing the window does not stop the download: it keeps
going in the downloads panel.

To choose for yourself, go to **Discover**, search for a model (`qwen`, `llama`,
`gemma`…) and open it. The quantization list is colour-coded for *your* machine:

- <span class="ow-verdict ow-verdict--gpu"></span> **green** — runs fully on the GPU;
- <span class="ow-verdict ow-verdict--split"></span> **yellow** — splits between GPU and CPU, slower;
- <span class="ow-verdict ow-verdict--cpu"></span> **grey** — CPU-only.

Pick one, download it, and it lands in **My Models**. Interrupted downloads are
resumable, even after restarting the computer.

::: tip Gated models
Some repositories require accepting a license on Hugging Face. The acceptance
is stored on **your account**, not on the machine — the app needs to know who
you are.

In **Settings**, click **Sign in with Hugging Face**: your browser opens the
authorization page, you confirm, and the account is connected. There is no
token to create or paste, and the app renews the session on its own while
downloading.

Then click **Accept the license on Hugging Face** on the model's notice. While
that tab is open, the app asks the Hub every few seconds whether the gate
opened — the moment you accept, the notice disappears and the download that
was waiting starts on its own.

If you would rather keep pasting a token, **Or use a token manually** is still
there. On that path the usual caveat applies: a *fine-grained* token needs the
"Read access to contents of all public gated repos" permission, or the download
fails even with the license accepted — and **Settings** tells you when that is
the case.
:::

## Tuning for this machine

Once a model is downloaded, OpenWeights can ask llama.cpp itself how much memory
each configuration costs **on your card**, recommend one with the numbers behind
it, apply it, and roll it back on its own if the model fails to load.

If you want, it then measures real tokens/s and replaces the estimate with what
your machine actually delivered. An estimate you can check beats a promise you
cannot.

## The status bar

The strip along the bottom is not decoration. It reports, live: CPU, RAM, GPU
and VRAM; the card's **power draw against its limit**; disk and network; **which
model is loaded** (with a pulsing dot while it generates); **how much of the
context window is in use**; and the current **tokens per second**.

That last number comes from the server, not from the chat — so it counts the
coding agent and any external app pointed at your API, not only what you type
here.

Clicking the model name lists every model the Local Server can serve, with
which ones are in memory; **Load** and **Unload** are right there, without
leaving the screen you are on. The chart icon at the far right opens the
hardware monitor.

## Simple and Advanced

A new install opens in **Simple** mode. The sidebar shows what you need to chat
with your models, in two groups: **Get started** (Chat, Discover, My Models)
and **Agents** (AgenticOw, OwCLI). The Local Server, the model sources and
Train stay out of the way, and the chat drops fine-tuning controls such as
reasoning effort.

Nothing stops working in Simple mode. A button that leads to one of those
screens still opens it, and the sidebar shows it while you are there. To see
everything, switch to **Advanced** under **Settings → Preferences → Mode**. If
you already used OpenWeights before this mode existed (the engine installed or
a model in the library), the app opens in Advanced and nothing moves.

## Command palette and shortcuts

**Ctrl+K** opens the command palette: type a few letters of what you want (a
screen, a tab such as *Local Server › Performance*, *New conversation*, *Start
the Local Server*, switching mode or theme) and press **Enter**. It is also how
anything hidden in Simple mode stays one keystroke away.

A few shortcuts work anywhere: **Ctrl+N** starts a conversation, **Ctrl+,**
opens Settings and **Ctrl+1…9** go to the screens in sidebar order. **?** shows
the full list. With focus in an OwCLI terminal, the keyboard belongs to the
program running there (Ctrl+K deletes to the end of the line in the shell), and
the palette opens with **Ctrl+Shift+K**.

## Where to go next

- [Models and quantization](/guide/models) — how to read the colours.
- [Chat](/guide/chat) — the conversation screen, parameters and attachments.
- [Chat performance](/guide/performance) — why an answer took that long.
- [The coding agent](/guide/harness) — when you want work done, not just
  answers.

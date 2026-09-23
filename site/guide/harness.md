# The coding agent

Chat is chat: the model talks back. When you want work done — files read and
edited, commands run, a project taken from one state to another — that is
**AgenticOw**, and it has its own item in the sidebar, right below Chat.

AgenticOw is OpenWeights' own fork of the
[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness) (MIT
license), kept at [pedro-canedo/agenticow](https://github.com/pedro-canedo/agenticow)
the way Cursor keeps VS Code: the upstream core, our layer on top — branding,
Portuguese, your models, privacy — and regular syncs with the upstream. It is
not a link to somewhere else: the app installs it, supervises it and shows it
inside its own window.

## The first open

AgenticOw arrives as a **prebuilt runtime**, compiled by our CI for each
system (Windows, Linux and macOS) — there is no `npm install` on your
machine. The first open downloads the package for your
system into an app folder, once, and checks it against the sha256 and size
**built into the app binary** before running anything. A package that does not
match is refused. It runs on the app's portable Node, never your system's.

After that it opens in seconds. You can leave the screen while it prepares;
the progress is there when you come back.

::: tip It is all inside the app
The runtime lives in an app folder, listens on loopback only, and shuts down
when OpenWeights does. Removing it is a button on the same screen — with the
choice of keeping or also deleting the sessions and settings created inside it.
:::

## Inside the main window

The AgenticOw interface is shown **inside the main window**, over the content
area, not in a separate window. Switching to Chat or Models and back does not
reload it: the open session keeps running, mid-task included. Dialogs from the
app appear on top of it.

It speaks the app language — Portuguese or English — and changing the
language in Settings changes AgenticOw too, without a restart.

## Updating

The AgenticOw version is pinned by each OpenWeights release, and it updates
**together with the app**: when OpenWeights updates, the next open downloads
the new runtime, verifies it the same way and starts it. The previous version
is only deleted after the new one has started successfully, so a bad download
never leaves you without an agent.

This is the reason for the fork. The harness the app used before came from npm
at a version fixed in the app, and newer releases changed how they start and
authenticate in ways the app could not follow — so it stayed behind. Building
the runtime ourselves makes updating it part of updating the app.

## What it already knows about your models

You do not configure a provider, paste a base URL or copy an API key. The app
hands AgenticOw everything it knows:

- **Local Server** — every model your llama.cpp router serves, each with its
  real context window.
- **OpenRouter** — your favourites, when the provider is on and has a key.
- **9router** — its catalogue, when it is installed and running.

And it keeps that list current **while it runs**: starting or stopping the
engine, downloading or deleting a model, turning Jev on or off, changing the
OpenRouter key or favourites, starting or stopping 9router — each one updates
the model picker without restarting anything.

API keys travel only to the AgenticOw process memory. They are never written to
a file, so a folder someone reads later has no secret in it.

## Privacy

The upstream project's telemetry is off: session telemetry, message and
command feedback, and the plugin inventory attached to requests to DeepSeek's
API are all disabled in our build, and a test in the fork checks after every
sync with the upstream that each of them is still off. Nothing goes to DeepSeek
unless you choose DeepSeek as a provider yourself.

## Coming from the DeepSeek Harness

If you used the DeepSeek Harness in an earlier version of the app, the first
open **copies** its sessions and settings into AgenticOw. The original folder
is left untouched, so going back to an earlier version still finds everything
where it was.

## Reasoning effort

Models that think before answering are offered an effort selector next to the
model name, and **the levels come from the model's own chat template**, not
from a list we made up. The template states which values it accepts and refuses
anything else; the app reads that line and offers exactly those.

This matters more than it sounds. Asked for something open-ended, a reasoning
model at its highest setting can spend its **entire output budget thinking** and
stop before writing a single file. Measured on a reasoning model with such a
template, on the same question: the low level produces around 600 characters of
reasoning, the highest produces nearly 6,000 — ten times more. Turning it down is often the
difference between an answer and a truncated draft.

**Off** really turns thinking off, not down.

With [Jev](/integrations/providers#jev-a-decision-layer) enabled for coding
agents, the selector still sets the ceiling, but each request AgenticOw sends
goes through a local proxy that decides, per message, whether the model should
think at all and at which of the template's levels. Continuations of the same
task (tool results coming back) reuse the decision instead of paying for it
again, and anything the proxy cannot decide passes through exactly as
AgenticOw sent it.

## Output cap

Each local model also declares how much it may write in one answer — half its
own context window. Without that, AgenticOw assumes a fixed 32k for every
model, which a small one cannot honour and a large one does not need to be
limited to.

If a reply ever stops with *Output token limit reached*, that is this cap, and
the reply so far is kept: sending `continue` resumes it.

## Speed, measured rather than promised

Speculative decoding — the model guessing several tokens ahead and checking
them in one pass — is on the **Local Server** screen, not here, because it is a
property of the server. It is worth knowing that the app measures it on your
machine and applies what wins, and that it verifies the answer did not change
before applying anything. See
[measured speculation](/integrations/local-api#measured-speculation).

## The other agents

Claude Code, Aider and OpenCode are not managed by the app, but they get a
ready-made command pointed at your local API, with the key masked in the
preview. They live under **Local Server → Open in a harness**. See
[the local API server](/integrations/local-api#open-in-a-harness).

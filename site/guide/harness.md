# The coding agent

Chat is chat: the model talks back. When you want work done — files read and
edited, commands run, a project taken from one state to another — that is the
**DeepSeek Harness**, and it has its own item in the sidebar, right below Chat.

It is not a link to somewhere else. OpenWeights installs it, supervises it,
configures it against your models, and runs it embedded in the app.

## The first open

Opening it the first time downloads a portable Node runtime and about 190 npm
packages into a folder of the app's own — never a global install, never your
system's Node. It takes **ten to thirty minutes**, most of it spent resolving
the dependency tree, and the screen shows the live log the whole way. You can
leave the screen; the install keeps going and the progress is there when you
come back.

After that it is one click.

::: tip It is all inside the app
The harness runs from an app folder, listens on loopback only, and shuts down
when OpenWeights does. Removing it is a button on the same screen — with the
choice of also deleting the sessions and credentials created inside it.
:::

## What it already knows about your models

You do not configure a provider, paste a base URL or copy an API key. Every
time the harness starts, the app writes its configuration with everything it
knows:

- **Local Server** — every model your llama.cpp router serves, each with its
  real context window.
- **OpenRouter** — your favourites, when the provider is on and has a key.
- **9Router** — its catalogue, when it is installed and running.

API keys never enter that file. It names environment variables, and the values
go only into the harness process — so a file someone reads later has no secret
in it.

## Reasoning effort

Models that think before answering are offered an effort selector next to the
model name, and **the levels come from the model's own chat template**, not
from a list we made up. The template states which values it accepts and refuses
anything else; the app reads that line and offers exactly those.

This matters more than it sounds. Asked for something open-ended, a reasoning
model at its highest setting can spend its **entire output budget thinking** and
stop before writing a single file. On a Qwen3.8 27B, measured on the same
question: the low level produces around 600 characters of reasoning, the
highest produces nearly 6,000 — ten times more. Turning it down is often the
difference between an answer and a truncated draft.

**Off** really turns thinking off, not down.

## Output cap

Each local model also declares how much it may write in one answer — half its
own context window. Without that, the harness assumes a fixed 32k for every
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

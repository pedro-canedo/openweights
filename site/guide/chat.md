# Chat

The chat screen is where a model answers you. It streams, renders markdown and
code, keeps history on disk, and lets you go back and change what you asked.

## The basics

- **Model** — picked in the composer. Switching mid-conversation is saved with
  the chat, so reopening it later restores the same model.
- **The engine starts on its own** at your first message; the first load of a
  model into memory takes a while, and the app says so instead of looking
  frozen.
- **Regenerate, edit and resend, delete** — every message has them. Editing a
  message rewrites history from that point.
- **Copy as Markdown** exports the whole conversation.
- **Read aloud** speaks an answer; the microphone button dictates one.

## Attachments and `@file`

Drop files onto the conversation, use the **+** menu, or type `@` to pick a file
from the project folder. Images require a multimodal model — the app warns you
when the current one has no vision projector.

## Context window

The ring next to the composer is the **context meter**: how much of the model's
window is already committed, broken down into system prompt, conversation,
reasoning, current message and attachments.

This matters more than it looks. Reasoning and the answer share the same budget
as the conversation, and the KV cache lives in VRAM: a bigger window is not
free. The window is set at **load time**, not per message — changing it asks you
to reload the model.

## Parameters

The panel on the right has two halves, and the split is the point:

**Per message** — take effect on the next send:

| Parameter | What it does |
|---|---|
| System instructions | The standing instruction for the model |
| Creativity (temperature) | Higher wanders more, lower repeats more |
| Top-P / Top-K | How wide the pool of candidate tokens is |
| Response token limit | Hard cap on the answer |
| Effort | More complete answers, slower and heavier |

**At load time** — need the model reloaded:

| Setting | What it does |
|---|---|
| Context window | How much the model can remember in this load |
| KV cache | Conversation memory on the GPU; compressing fits a larger window in the same VRAM |
| Flash attention | Faster generation at the same memory |
| Speculation | Predicts tokens ahead — MTP when the file ships it, n-gram helps on code |
| Vision | Whether the projector is loaded always, on demand, or never |
| GPU layers, experts on CPU, batch, threads, mmap, mlock | The llama.cpp knobs, with plain-language hints |

Presets save a set of parameters under a name.

## Chat or agent

The toggle in the composer switches between **Chat** — the model only talks —
and **Agent**, where it can use tools to actually do the work. That is a
different screen's worth of behaviour, and it has its own section:
[the agent harness](/agent/).

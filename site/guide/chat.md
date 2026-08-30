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

Presets save a set of parameters under a name.

**Load-time settings** — context window, KV cache, flash attention, speculation
(MTP), vision and the rest of the llama.cpp knobs — moved house: they now live
in **Local Server**, next to the model that uses them. The shortcut in the panel
takes you straight there with the conversation's model already selected. The
reason: how a model loads is a property of the model, not of the conversation —
it is the same configuration for chat and for any app consuming the API. See
[configuring llama.cpp](/integrations/local-api#configuring-llama-cpp).

## When you want work done, not just answers

Chat is chat: the model talks. For agent work — reading and editing files,
running commands — the **Agent** button in the composer opens the
[DeepSeek Harness](https://github.com/deepseek-ai/deepseek-harness): a full
coding agent that the app installs, configures and opens for you, already
pointed at every provider and model you have. The first open installs it;
after that it is one click. See
[Open in a harness](/integrations/local-api#open-in-a-harness).

# Configuration reference

Everything the app can tell llama.cpp, and where each setting ends up. This
page explains the **system**; the exact list of switches lives in the app,
under **Local Server → Performance → Configure llama.cpp**, because it is read
from the binary this release pins and grows when llama.cpp grows.

## Where a setting can live

The engine runs in Router mode: one `llama-server` process, plus an INI file
that describes each model. A setting therefore has two possible homes — the
process command line, and the INI — and which one it uses is not cosmetic.

| Scope | Where it lands | Example |
|---|---|---|
| **Global** | Command-line argument of the server process | Port, host, log level |
| **Per model** | The model's own section in the Router INI | Context size, GPU layers, quantization of the cache |
| **Both** | The `[*]` section of the INI when set globally, never the command line | Flags that make sense as a default but must stay overridable |
| **Router only** | A key that exists only in the INI | `load-on-startup` |
| **Managed** | The app decides, and shows a lock | Port, API key, cluster wiring |

**Precedence is what makes "Both" subtle.** The Router resolves a setting in
this order:

```
command line   >   the model's own section   >   the [*] section
```

A global flag placed on the command line would therefore beat every per-model
choice, silently. That is why "Both" settings go into `[*]` instead: a default
that a model can still override is a default; one that cannot is a rule
pretending to be a default.

## Categories

The catalogue is grouped by the question you arrive with, not by the order
`--help` prints:

| Category | What it covers |
|---|---|
| **context** | Context window, batching, how much of the prompt is kept |
| **memory** | GPU layers, memory locking, cache quantization, offload |
| **cpu** | Threads for generation and for prompt processing |
| **rope** | RoPE scaling and YARN, for stretching a model past its trained window |
| **spec** | Speculative decoding: the draft model and how far it guesses ahead |
| **multimodal** | The projector (`mmproj`) for models that see images |
| **adapters** | LoRA and control vectors |
| **server** | Host, port, slots, timeouts, log level |
| **router** | Keys that only the Router understands |
| **usage** | Everything else the pinned binary accepts |

## Settings that depend on something else

Some switches only make sense in a context, and the app hides them until it
holds: a draft-model setting is meaningless without speculation enabled, and a
projector path is meaningless on a model that cannot see. The requirements the
app checks are a GPU being present, more than one GPU, the model being a
Mixture-of-Experts, the model supporting multi-token prediction, a projector
being present, speculation being on, RoPE being set to YARN, and Flash
Attention being on.

This is why the same model shows a different list on two machines. Nothing is
missing — the switch would have no effect there.

## Load-time versus request-time

A setting that the engine reads when it **loads** a model cannot change while
that model is loaded. The app says so, and stopping and starting the server is
part of applying it. Context size, GPU layers and cache quantization are all
load-time.

Request-time parameters — temperature, top-p, penalties, the reply cap — belong
to the conversation, not to the engine, and live in [Chat](/guide/chat).

## Seeing what will be sent

The **Configure llama.cpp** card previews the exact command line and the exact
INI the app will write, rendered by the same code that writes them at startup —
not a reconstruction. If the preview and the reality ever disagree, that is a
bug worth reporting.

## Measuring instead of guessing

A configuration that looks better on paper is a hypothesis.
[Chat performance](/guide/performance) explains how to test one against the
current profile with warmup and repetitions, and how the app decides when a
difference is real rather than noise.

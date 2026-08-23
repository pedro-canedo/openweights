# Models and quantization

## GGUF, and only GGUF

OpenWeights runs models through llama.cpp, which reads **GGUF** files. MLX
(Apple's format), safetensors, GPTQ and AWQ do not run here. When you land on a
model that only publishes one of those, the app says so and offers to look for
the GGUF version — for popular models, someone has almost always published one.

## What quantization actually costs you

Quantization is how many bits each weight keeps. Fewer bits means a smaller
file, less memory and faster generation, at some loss of quality:

| Quantization | Rough size vs. F16 | Typical use |
|---|---|---|
| `Q8_0` | ~50% | closest to the original, if it fits |
| `Q6_K` | ~40% | very close, common on 24 GB cards |
| `Q5_K_M` | ~33% | good balance |
| `Q4_K_M` | ~28% | the usual default — most quality per GB |
| `Q3_K_M` | ~22% | when nothing else fits |
| `Q2_K` | ~16% | last resort, visible degradation |

The rule that matters is simpler than the table: **the whole model in VRAM beats
a better quantization split with the CPU**. A `Q4_K_M` fully on the GPU is
usually faster *and* more pleasant than a `Q6_K` spilling into system RAM.

## Reading the colours

In the quantization drawer each file is scored against your actual hardware:

- <span class="ow-verdict ow-verdict--gpu"></span> **Green** — fits fully in VRAM, with room for the context window.
- <span class="ow-verdict ow-verdict--split"></span> **Yellow** — part of the layers go to the CPU. It works; it is slower.
- <span class="ow-verdict ow-verdict--cpu"></span> **Grey** — CPU-only. Usable for small models, painful for large ones.

The score accounts for the context window too, because the KV cache lives in the
same memory: a model that fits at 4k tokens may not fit at 32k.

## My Models

Everything downloaded shows up in **My Models**, with size and quantization, a
shortcut to chat, and delete. Models imported by hand — dropped into the folder
without going through Discover — appear marked as such.

Downloads that stopped halfway are listed apart, with **Resume** and
**Discard**. Resuming survives closing the app and rebooting the machine.


## Tuning, without you tuning anything

Every knob llama.cpp exposes — context size, KV cache type, flash attention,
how many layers go to the GPU, how the load splits across two machines — has a
right answer for *your* hardware and this model. The app finds it on its own.

It does not guess. Three things are asked rather than estimated:

| Question | Who answers |
|---|---|
| Which devices exist and how much is free on each | `llama-server --list-devices`, including the borrowed GPU when a pair is up |
| How many layers this model really has | the GGUF header (`block_count`) |
| How much memory a configuration costs | `llama-fit-params`, in about a second and a half, without loading the model |

With those three, a directed search converges on a configuration: the largest
context that still keeps the weights on the GPU, compressing the KV cache only
when the window demands it, flash attention on where it helps, and the split
ratio taken from real free memory.

It runs by itself in the background once the engine is up, and again when the
device set changes — pairing with another machine changes what fits as much as
swapping a card. Each model remembers the fingerprint of the situation it was
tuned for (machine, engine version, devices), so nothing is measured twice.

**What you chose by hand is never touched.** The automatic pass only fills what
it left itself, or what was never set. And because the Router preset is read at
boot, a configuration found while the engine is running takes effect the next
time it starts — the screen says so.

### When you want to see the numbers

**My Models → Tune for this machine** opens the panel: every candidate with the
memory it costs per device, which one was picked and why, and the option to
measure real tokens/s with `llama-bench` instead of trusting the estimate. It
is there for when you want to look, not because the app needs you to.

From there, **Full configuration** takes you to the Local Server screen, where
the same configuration is editable flag by flag — including the ones this panel
never proposes. See
[configuring llama.cpp](/integrations/local-api#configuring-llama-cpp).

::: tip One thing we cannot do for you
On Apple Silicon, macOS caps Metal at about 75% of unified memory. Raising it
needs a terminal and your password, so the app shows the command instead of
running it.
:::

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

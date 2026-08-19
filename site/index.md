---
layout: home

hero:
  name: OpenWeights
  text: Models. Your machine. Your rules.
  tagline: An open-source desktop app that runs LLMs locally — and an agent harness that actually gets work done on small models.
  image:
    src: /logo-1024.png
    alt: OpenWeights
  actions:
    - theme: brand
      text: Download
      link: /guide/install
    - theme: alt
      text: What it is
      link: /guide/
    - theme: alt
      text: The agent harness
      link: /agent/

features:
  - icon: 🔍
    title: Hardware on autopilot
    details: Detects CPU, RAM, GPU and VRAM, then downloads the llama.cpp build that matches — CUDA, Vulkan or CPU-only. No terminal, no CUDA setup.
    link: /guide/first-run
    linkText: First run
  - icon: 🤗
    title: Models already filtered
    details: Searches GGUF on Hugging Face and recommends the quantization for your PC. Green runs fully on the GPU, yellow splits with the CPU, grey is CPU-only.
    link: /guide/models
    linkText: Models and quantization
  - icon: 🤖
    title: An agent that answers to you
    details: It reads and edits files, runs commands, uses Git and browses the web — every action through the authorization level you chose, with a checkpoint before the first change.
    link: /agent/
    linkText: How a run works
  - icon: ⚡
    title: Code Mode
    details: Instead of asking for one tool at a time, the agent writes a program that uses them all at once. Measured here - 3.4x faster and 7.4x fewer round trips than the native loop.
    link: /agent/code-mode
    linkText: The measurement
  - icon: 🧠
    title: Memory and project index
    details: The agent keeps what it learned between chats and searches your code by meaning, not just by string.
    link: /agent/memory
    linkText: Memory
  - icon: 🔌
    title: OpenAI-compatible API
    details: Point any other app at localhost and use the same model. Optionally reachable from your local network.
    link: /integrations/local-api
    linkText: Local server
---

<div style="max-width: 780px; margin: 4rem auto 0; text-align: center;">

## Nothing is sent to a server of ours

There is no server of ours. Models run on your machine, conversations are stored
in a local SQLite file, and the only network traffic OpenWeights starts on its
own is downloading the engine, the models you pick, and checking whether a new
version of the app exists.

If you point it at an external provider — OpenRouter or 9router — that traffic
goes where you told it to, and the screen says so.

</div>

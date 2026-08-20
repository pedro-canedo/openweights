# Extra GPU on the network

Two machines running OpenWeights on the same network can load one model
together. The model file stays on one of them; the other only lends its GPU.

It is the answer to a specific problem: a model that fits in *neither* card
alone. A 12 GB card and an 18 GB Mac are 30 GB when paired — enough for weights
that would otherwise spill into system RAM and crawl.

## What it is not

- **Not faster.** Splitting a model over Ethernet adds latency at every layer
  boundary. Paired, a model that already fit on one card gets *slower*. The app
  knows this and only splits the models that don't fit locally.
- **Not a cloud.** Nothing leaves your network. There is no account, no relay,
  no server of ours in the middle.
- **Not more than one helper.** One host, one worker. That is the whole design.

## Requirements

| | |
|---|---|
| **Same llama.cpp tag** | The RPC wire format changes between builds. Different tag, and the pairing is refused with *different tag — update* |
| **The RPC engine** | The official llama.cpp packages are built without RPC. The app downloads an overlay build for the same tag when you turn the feature on |
| **A GPU on both sides** | A machine with no GPU appears in the list marked *no GPU* and cannot be paired |
| **Same network** | Discovery is mDNS; the control channel is a TCP port on your LAN |

The download is automatic and matched to your machine, exactly like the
llama.cpp runtime the app fetches on first run: it looks at your GPU, picks the
build that fits and installs it beside the engine. There is nothing to place by
hand.

Builds exist for **Windows with CUDA**, **Windows with Vulkan** (AMD and Intel)
and **macOS on Apple Silicon**. A machine with no GPU has nothing to lend and
nothing to split, so there is no build for it — the panel says so instead of
downloading something useless.

## Turning it on

The panel lives in **Local Server**, under the server settings.

**Offer GPU on the network** is off by default, on both machines. Nothing is
announced and no port is opened until you turn it on — turning it on is the
consent. Do it on both machines, on a network you trust.

The first time, the app fetches the RPC engine (a few hundred MB). The panel
says *preparing the RPC engine…* while that happens.

## Pairing

1. On the machine that has the model, the other OpenWeights appears in the list
   with its GPU and how much memory it offers.
2. Press **Use as extra GPU**.
3. The other machine gets a desktop notification and shows the request in its
   own **Local Server** panel. Nothing happens until someone presses **Accept**
   there. That is the only moment a human decision is required.
4. On accept, the helper starts its RPC process and the host restarts its engine
   pointing at it. The status bar shows a chip on both sides.

A pair that was accepted once reconnects on its own the next time both apps are
open on the same network — using a secret agreed during that first accept, not
the name broadcast on the network. **Forget** drops it, and the next attempt
asks again.

## How the split is decided

Each machine announces a budget, not its whole card: **75% of VRAM** on NVIDIA,
or **75% of 75% of system RAM** on Apple Silicon (macOS itself refuses to give
Metal much more than three quarters of unified memory). The rest is the KV cache
and compute buffers — announcing everything is the classic way to run out of
memory on the first prompt.

The device with more memory takes the first layers. An 18 GB helper next to a
12 GB local card becomes `--device RPC0,CUDA0 --tensor-split 3,2`. The panel
shows the ratio it chose.

## What to expect while it runs

- **The first load is slow.** The weights travel to the other machine over the
  network — around two minutes for 16 GB on gigabit Ethernet. The helper caches
  them on disk, so loading the same model again is fast.
- **Wi-Fi works, badly.** Every layer boundary crosses the link. Cable if you
  can.
- **The helper should be idle.** Lending the GPU while chatting on the same
  machine books the same video memory twice. The app refuses to lend while your
  local server is running, and says so.
- **If the other machine disappears**, the pair is dropped within about fifteen
  seconds and the host restarts its engine without the remote GPU.

## Security

::: danger Read this before turning it on
The RPC channel has **no password**, and llama.cpp says so plainly: the RPC
backend is a proof of concept, and it should never run on an open network. The
pairing secret protects you from a stranger who merely knows the name your app
broadcasts — it is not encrypted, so it does not protect you from someone
capturing traffic on the same network.

Only accept machines you recognise, and never forward the port to the internet.
:::

The parts that are ours behave conservatively: the feature is off until you turn
it on, the helper opens the RPC process only after a human presses Accept, an
unsolicited "I accept" from a machine you did not ask is refused, and a request
carrying the wrong secret cannot displace a live pair.

## When it doesn't work

| What you see | What it means |
|---|---|
| *No other OpenWeights visible on this network* | The other machine has the feature off, is on another network, or mDNS is blocked by the firewall |
| *different tag — update* | The two apps ship different llama.cpp builds. Update both |
| *This engine build does not include RPC yet* | The overlay for this tag hasn't been downloaded — or hasn't been published for your GPU |
| *the local server is running on this machine* | Stop the local server on the helper before lending its GPU |

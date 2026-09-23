# External model sources

**Model sources** answers one question: where do your conversations get
answered? Your own machine is the default and needs nothing here. The rest of
the screen is for when you want something else.

At the top, one card per source says which are ready and, for the ones that are
not, why — and clicking a card takes you straight to the tab where that gets
resolved.

## OpenRouter

Hundreds of models behind one key.

The **catalogue is public** — you can browse it, with price per million tokens,
context size and whether the model supports tools, before deciding anything. The
key is only needed to actually chat. Filter to free models only or to what you
have already pinned, search by name or id, and pin the ones you use: **only
pinned models show up in the chat's model picker**, and the card says how many
those are and how many models the current filter found.

With a key set, the screen shows what you have spent and your credit limit.

## Decisions — the reflex in front of your models

The **Decisions** tab holds a small, fast layer that runs before each message:
a *decider* looks at the current message plus a short tail of the conversation
and answers **how much reasoning the message needs** — none, medium or high.
The app then turns the local model's thinking on or off per message instead of
using the conversation's fixed effort. A greeting stops paying thirty seconds of
thinking; a logic problem still gets the full budget.

A decider does not write text. It is asked one question per field, with the
allowed values spelled out, and only the first token of each allowed value is
scored — all fields in parallel, from a prefix (instructions plus schema) that
stays cached. That is why an answer lands in milliseconds and can never fall
outside the schema: the decider can misjudge, never malform.

Two deciders exist, tried in order. Everything is fail-open: when neither
answers, the effort you configured stands.

### Local decider

A second `llama-server`, built from the
[parallel-decision fork of llama.cpp](https://github.com/thecodacus/llama.cpp/tree/parallel-decision),
with a small model dedicated to deciding — by default
`Qwen/Qwen2.5-1.5B-Instruct-GGUF` in Q8_0 (1.9 GB, Apache-2.0), attention-only,
which is the architecture this technique rewards. **Install** downloads the
engine (about 200 MB, a package OpenWeights builds and publishes itself) and the
model; both show up in the downloads panel. You can pick any other GGUF from
your library as the decider afterwards.

It starts together with the local server, on `127.0.0.1:11713`, and stops with
it. While it runs it holds about **2.6 GB of VRAM** next to the chat model, and
the fit measurements do not account for that: it switches itself on only on
GPUs with 12 GB or more, and below that the card shows the cost and lets you
switch it on by hand. It needs Windows or Linux with an NVIDIA GPU on CUDA 13
(driver 580 or newer). Nothing leaves your machine.

### Fallback: Jev on OpenRouter

The **Jev** decision model from TypeSafe, reached through the OpenRouter key,
answers when the local decider is not installed, is still loading, or fails.
When it does, the message and tail leave your machine — the card says so next
to the switch — and the cost is a fraction of a cent per thousand messages
(input tokens only). Uncheck the fallback to keep decisions strictly local.

### Where it applies, and the endpoint

Two switches say where the decision applies. **Chat in the app** decides before
each send and shows who decided in the answer's run details (*Local* or *Jev*).
**Coding agents** starts a small local proxy on `127.0.0.1:11712` in front of
the engine; AgenticOw and the other agents the app launches are pointed at
it, and every `chat/completions` they send gets the decision applied
to the body. The response header `x-openweights-jev` says what happened:
`alto;0.91;local`, `medio;0.80;jev`, `nenhum;cache` or `default;<reason>`.
Every other request passes through untouched, streaming included, and the proxy
only listens on the loopback address.

The proxy also forwards `POST /v1/decision` to the local decider, so anything
that can reach `127.0.0.1:11712` — a coding agent, the gateway, the
[decision playground](https://github.com/thecodacus/decision-playground) — can
ask its own questions. A request names its fields with allowed values and one
or more contexts; the answer carries each field's value and probability:

```json
POST /v1/decision
{
  "instructions": "Route this support ticket.",
  "schema": {
    "category": {"type": "enum", "choices": ["billing", "technical", "other"],
                 "description": "What is the ticket about?"},
    "urgent": {"type": "boolean", "description": "Does it need urgent handling?"}
  },
  "contexts": ["I was charged twice and need this fixed today."]
}
```

```json
{"results": [{"decision": {"category": "billing", "urgent": true},
              "fields": {"category": {"value": "billing", "probability": 0.97},
                         "urgent": {"value": true, "probability": 0.88}}}],
 "timings": {"total_ms": 41.0}}
```

Fields cannot see each other's answers: ask independent questions. **Only act
above** is the confidence floor, and **Test decision** sends one sample question
and reports which decider answered and how long it took.

## 9router

A local router with its own dashboard: it puts accounts from several providers
behind one address.

OpenWeights installs it in an **isolated folder** — portable Node included,
nothing touching your system — runs it, and removes it when you ask. Its models
and combos show up in the chat model selector, with the API key obtained from
9router itself. The process dies with the app.

::: warning Installing takes a while
It downloads portable Node.js and the 9router package — a few hundred MB on
disk. With antivirus active on Windows it can take 2 to 10 minutes.
:::

The dashboard **opens in its own window**. That is not a stylistic choice:
embedded in the app screen, its login never completes. The first boot password
is shown in the app; after that, the password you set inside 9router wins.

Uninstalling asks whether to keep the data — deleting it removes the accounts
and providers configured inside 9router, and that cannot be undone.

## Gateway — a single entry point

One address that forwards to the local engine and to 9router **by prefix**:

| Prefix | Goes to |
|---|---|
| `/local` | The local llama.cpp engine |
| `/9router` | 9router, when it is running |

Useful when you want to point another tool — an editor, a script — at
OpenWeights without memorising two ports. It runs a local Traefik, pinned to a
fixed version.

**What it does not do**, so nobody expects it: it does not create a tunnel to
the internet (Traefik is a reverse proxy, not a tunnel); it does not merge the
catalogues into a single `/v1/models`, because that would be our code and not
routing; and it adds no authentication of its own.

It is **optional and off by default** — nothing in the chat depends on it.

::: warning Exposing to the local network
With *accept connections from the local network* on, any device on your network
reaches your models without a password. Only enable it on a network you trust.
:::

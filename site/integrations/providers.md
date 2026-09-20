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

## Jev — a decision layer

Below the OpenRouter card sits **Jev**, a decision model from TypeSafe that the
app reaches through the same OpenRouter key. It does not generate text: it
receives the current message plus a short tail of the conversation and answers,
in well under a second, **how much reasoning the message needs** — none, medium
or high. The app then turns the local model's thinking on or off per message
instead of using the conversation's fixed effort. A greeting stops paying thirty
seconds of thinking; a logic problem still gets the full budget.

It is off by default and only becomes available once the OpenRouter key is set.
When it is on, that message and tail leave your machine — the card says so where
you switch it on. Everything else stays local, and the cost is a fraction of a
cent per thousand messages (input tokens only; the answer is free).

Two switches say where it applies. **Chat in the app** decides before each
send and shows the choice in the answer's run details. **Coding agents** starts
a small local proxy on `127.0.0.1:11712` in front of the engine; the DeepSeek
Harness and the other agents the app launches are pointed at it, and every
`chat/completions` they send gets the same decision applied to the body. Every
other request passes through untouched, streaming included. The proxy only
listens on the loopback address: anything reaching the engine from the network
bypasses it.

**Only act above** is the confidence floor. Below it — or whenever Jev is
unreachable, the key is missing, or the answer contradicts itself — nothing
changes and the effort you configured stands. **Test decision** sends one
sample question so you can see the key and endpoint working before a
conversation depends on them.

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

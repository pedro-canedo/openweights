# API reference

The local server is `llama-server` in Router mode, so the surface is the one
llama.cpp exposes — OpenAI-compatible where it matters. This page lists what
you can call and how Router mode changes the answers. For turning the server
on, the address and the API key, see the
[local API server](/integrations/local-api).

Every path below is relative to the address shown at the top of **Local
Server** — `http://127.0.0.1:<port>` by default.

## Authentication

If an API key is set, every request must carry it:

```
Authorization: Bearer <your key>
```

Without a key set, requests are accepted from the machine itself. Turning on
local-network access removes that boundary — see the warning in
[local API server](/integrations/local-api).

## Chat completions

```
POST /v1/chat/completions
```

The endpoint an OpenAI client expects. The `model` field is not optional here
the way it is against a single-model server: **it is what tells Router mode
which model to load**.

| Field | Notes |
|---|---|
| `model` | The id from `GET /v1/models`, or the name shown in **My Models**. Loads the model if it is not resident |
| `messages` | Standard roles: `system`, `user`, `assistant` |
| `stream` | `true` streams server-sent events, which is what the chat screen uses |
| `temperature`, `top_p`, `top_k` | Sampling; the app's chat parameters map to these |
| `max_tokens` | Reply cap. The app also enforces its own — see [Chat](/guide/chat) |
| `tools` | Passed through to the model's chat template when the model supports it |

A model that has to be loaded first makes the request take longer, not fail.
That wait is the "loading" phase described in
[Chat performance](/guide/performance).

## Text completions

```
POST /v1/completions
```

The older, non-chat shape. Available for clients that predate chat completions;
new integrations should use `/v1/chat/completions`, because it is what applies
the model's chat template.

## Embeddings

```
POST /v1/embeddings
```

Works when the loaded model produces embeddings. A chat model asked for
embeddings answers with an error rather than a meaningless vector.

## Listing models

```
GET /v1/models
```

Returns the ids the Router is prepared to serve. These are the ids to put in
`model`, and they match the names in **My Models**. Asking this does **not**
wake a sleeping model.

## Server state

```
GET /health
GET /props
GET /slots
```

`/health` answers as soon as the process is up, which is what the app waits for
when you press Start.

`/props` reports the server's capabilities, including what the loaded model's
chat template accepts — that is where the app reads which reasoning-effort
levels a model really supports, instead of offering invented ones.

::: warning /props in Router mode
Asking `/props` **without** `?model=` returns the properties of the *router*,
not of any model. It is a real answer to a different question, and it is an
easy way to conclude that a model lacks a capability it has.
:::

`/slots` reports the context slots in use. The app polls it for the context
gauge and for tokens per second — the numbers there count every client,
including the coding agent and anything external you pointed at this address,
not just the chat.

## What Router mode changes

One process serves every model. That is what lets the chat and this API share
an engine instead of spawning one process per model, and it has three
consequences worth knowing:

- **`model` selects, and can load.** A request for a model that is not resident
  loads it. With **Concurrent models** set to 1, that unloads the previous one.
- **Server-wide readings are server-wide.** `/slots` and the speed figures
  include every client.
- **Some settings are load-time.** Context size and GPU layers apply when a
  model loads; changing them means the model reloads. See the
  [configuration reference](/integrations/configuration).

## Errors

Errors come back in the shape OpenAI clients expect, with an HTTP status and a
JSON body. The two you will actually meet:

| Status | Usually means |
|---|---|
| `401` | An API key is set and the request did not present it |
| `404` | The `model` id does not exist — check `GET /v1/models` |
| `500` | The engine refused the request. A common cause is a parameter the model's chat template does not accept, such as a reasoning-effort level it never declared |
| `503` | The server is up but the model is still loading |

The full log is in **Local Server → Advanced**, and it is the fastest way to
tell a rejected request from a model that failed to load.

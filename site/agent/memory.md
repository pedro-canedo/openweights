# Memory and project index

Two different things, often confused. Memory is what the agent *learned*. The
index is what your project *contains*.

## Memory

Every conversation starts from zero. Without memory you explain for the fifth
time that this project uses pnpm, that the test command is `cargo test -p lr_x`,
that you want short answers.

The lazy fix would be to dump the whole history into a vector database and
retrieve chunks by similarity. This is deliberately **not** that.

Memory here is **little, short, curated and inspectable**:

- **few facts, not whole conversations** — a local model's context is
  expensive, and what goes into the prompt goes into *every* following run;
- **every fact is curated before it exists** — normalized, length-limited,
  deduplicated, scoped;
- **everything is a file you can read**: `.openweights/memory/*.md` in the
  project, plus a global scope. Open the folder from the app, edit by hand,
  commit it if you want;
- **the heavy work happens when idle** — turning past runs into durable facts
  runs between runs, not in the middle of one. **Tidy memory now** triggers it.

The agent saves facts itself with `memory_save`, and you can add or forget one
by hand in the Memory panel. Each fact is either **global** or **this project**.

Do not confuse this with `.openweights/progress.md`, the scratch pad of *one*
run — what already ran, the files touched, the next step. It is rewritten
constantly and curated by nobody; it is described in
[work modes and plans](/agent/plans).

## Project index

`grep` only finds what you knew how to spell. "Where do we validate the session
token?" has no obvious keyword — that is what semantic search is for. But
embeddings alone are bad at proper nouns (`RagHandle`, `AGENT_MAX_STEPS`), where
literal matching is unbeatable.

So the search is **hybrid**: FTS5 (BM25) and vectors run in parallel and the
results are fused with RRF. The result list says which side found each snippet —
*text*, *meaning*, or both.

### Building it

**Index project** in the explorer column scans and chunks the files, then builds
vectors. Progress is shown per file and per chunk, and it is cancellable.

- **The vectors come from your own llama-server**, through `/v1/embeddings` —
  no download, no external service. Download an embedding model (`nomic-embed-text`,
  `bge-m3`) for this half to work.
- **Without an embedding model the index still works**, text-only. Worse, but
  working — the app says so instead of failing.
- **It is incremental.** A catalogue of hashes and mtimes per file avoids
  re-reading the whole project on every refresh.

Clicking a result opens the file in the app's editor, at the snippet. The agent
reaches the same index through the `workspace_search` tool.

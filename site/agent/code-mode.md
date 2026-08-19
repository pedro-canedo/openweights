# Code Mode

Instead of asking for one tool per step, the agent writes a **program** that
uses them all at once. The harness runs it, and only what the program prints
comes back to the conversation.

Turn it on in the composer, next to the agent toggle. It needs Node on the
machine — the app can install a portable one.

## One step, many calls

In native mode each tool costs a step: the model asks, the server reprocesses
the whole conversation, the result piles onto the window. Most of the wall-clock
time of an agent run is that reprocessing, not the tools.

In Code Mode the model spends **one** step writing the program, and the harness
executes as many calls as the program makes — none of them going through the
model. What comes back is what the script printed.

```js
// The tools are functions. They all return text and all need `await`.
const files = await fs_glob({ pattern: "logs/*.log" });
let total = 0;
for (const file of files) {
  const text = await fs_read({ path: file });
  total += (text.match(/ERROR/g) ?? []).length;
}
say(`${total} errors across ${files.length} files`);
```

The tool description shown to the model does not mention "Code Mode" or
architecture — it describes what to do. Small models follow an example, not a
concept.

## What does not change

Every call coming out of the script travels the **same path** a normal call
does: policy, confirmation, project snapshot, run trail, counters. The dispatch
is a trait implemented by the same tool runner, deliberately — a shortcut here
would erase, in one move, the protections the harness took six phases to earn.

The program itself is sandboxed: no file or command access except through the
tools.

## What was measured here

Measured on this machine, not quoted from someone else's slide. The case: 12 log
files, counts per level, an error percentage in a sorted `resumo.csv`, and a
`criticos.md` listing only the files above 25% errors.

**2026-08-18 · qwen2.5-coder:14b · RTX 5060 Ti**

| Mode | Steps | Tool calls | Time | Checks passed |
|---|---:|---:|---:|---:|
| native | 37 | 34 | 390.1 s | 4/9 |
| **program** | **5** | 17 | **115.5 s** | **5/9** |

- **3.4× faster** and **7.4× fewer round trips** — each round trip reprocesses
  the whole conversation, and that is where most of the time goes.
- Code Mode was the only one of the two that produced `resumo.csv` with all 12
  rows.
- **Neither finished the whole task.** The `app-12` percentage and `criticos.md`
  are still wrong in both. That is the 14B model, not the harness — the same
  limit the video that motivated this work reported with a cheaper model.

### Reproduce it

```bash
# an OpenAI-compatible server with the model loaded (here: Ollama)
OLLAMA_HOST=127.0.0.1:11435 OLLAMA_CONTEXT_LENGTH=32768 ollama serve &

cd src-tauri
OW_LIVE_URL=http://127.0.0.1:11435 OW_LIVE_MODEL=qwen2.5-coder:14b \
  cargo test -p lr_agent --test live_model -- --ignored --nocapture \
  code_mode_and_native_mode_run_the_same_case
```

The fixture has known counts, so the checks are objective — a separate test
guards the ruler itself.

## The four failures that got us there

The first four measurements failed, and each one pointed at a defect of ours:

1. **`run_code` refused as an unknown name** — the model wrote the right call in
   text, but the tool does not live in the active menu (it sits apart, with the
   run's signatures in its description).
2. **Broken JSON** — the model wrapped the program in `{"code": "..."}`, and a
   string with quotes and newlines never closes. The nudge now asks for a plain
   ```` ```js ```` block.
3. **The tool returned a sentence, not data** — `for (const f of await
   fs_glob(...))` iterated over `"12 files matched…"`, character by character.
   Tool output grew a `data` field, and the program started receiving arrays and
   raw text.
4. **Model guard-rails applied to the program** — the repetition detector
   escalated the run when the second program repeated a call from the first.

None of the four showed up without running against a real model.

# How a run works

Turn the **Agent** toggle on in the composer and a message stops being a
question: it becomes a *run*. The model gets a menu of tools, works step by
step, and every action passes through the authorization level you chose.

This section documents the harness — the part of OpenWeights that turns a
local model into something that gets work done.

## The problem the harness is built around

The audience for this app runs local models. An 8B model with an 8k context
window is not a smaller GPT-4: it loses the thread, it repeats itself, it picks
the wrong tool when shown thirty of them, and it happily claims it wrote a file
it never wrote.

Every design decision below exists because of that. None of them is a wrapper
around a big cloud model's good behaviour.

## A step

One step is one round trip to the model. The model answers with text, with one
or more tool calls, or with both. For each tool call the harness:

1. **asks the policy** whether it runs, asks you, or is refused
   ([authorization](/agent/authorization));
2. **takes a snapshot of the project** before the first change of the run
   ([checkpoints](/agent/checkpoints));
3. **runs the tool** and records the call, arguments, result and duration in
   the run trail;
4. **feeds the result back** to the model as the next step's input.

The run ends when the model says it is done, when you stop it, or when one of
the guard-rails below fires.

## The guard-rails

All of them are deterministic — no second model judging the first, nothing that
needs the network. In order of importance:

| Guard-rail | What it does |
|---|---|
| **Step budget** | A hard ceiling on steps. The run always ends. Hitting it says so and offers to continue. |
| **Error streak** | Three failures in a row and the run stops and hands the decision back to you, instead of insisting. |
| **Repetition** | The same call three times over is a loop, not progress. |
| **Re-read ledger** | Handing the model a file it already has only burns context. |
| **Context budget** | At ~80% of the window the history is summarized so the run can keep going. The trail says *"Context summarized to keep going."* |

There are also two clocks around the model itself: a generous one for the first
token — processing an 8k prompt on CPU takes minutes and that is not a hang —
and a tighter one between chunks, because silence after generation started
means a stuck server.

## Verification

When the run finishes, a cheap check runs over what it claims to have done. No
model involved: do the files it wrote exist? did any command exit with an error?

The goal is not to audit the work — it is to catch the easy lie, the one small
models tell most: announcing a file that the tool never created. The trail shows
**Result verified** or **Verification found problems**.

## Delegation

When the agent needs to *find out* something ("where is model routing decided"),
reading six files fills its window with raw content and leaves no room to think.

So it can delegate: a helper starts from zero — its own system prompt, its own
tool menu, empty history — investigates, and returns only a summary. The parent
pays ten lines instead of twenty thousand tokens.

The helper is not a separate run. It reuses the same loop and the same run
handle, which is why cancelling and approving keep working inside it, and why
the trail stays in one place. One helper at a time, on purpose.

## The run trail

Everything above is visible. The **Run** panel on the right of the chat shows
steps, tool calls with arguments and output, thinking blocks, checkpoints,
notes, the plan when there is one, and the closing counter — *N steps · M tools
· Xs*. Old runs are re-openable from the **Activity** screen.

## Next

- [Authorization and permissions](/agent/authorization) — who decides what runs.
- [Tools](/agent/tools) — the catalogue, by family.
- [Work modes and plans](/agent/plans) — how big tasks get cut down.
- [Code Mode](/agent/code-mode) — one step, many calls.

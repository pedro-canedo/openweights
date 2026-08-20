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
| **Context budget** | At ~80% of the window the history is summarized so the run can keep going, and a single message past a fifth of the window is clipped before that. The trail says *"Context summarized to keep going."* |
| **Proof per delivery** | No step closes on the model's word: the acceptance command has to exit 0, the promised files have to exist, and whatever has no mechanical proof goes through a criterion judge. Failed, the step goes back to the queue with the reason. |
| **A step that only talks** | Announcing work without running anything counts as stalled. Three of those earn the nudge; on the fifth the run stops and hands the decision back to you. |

There are also two clocks around the model itself: a generous one for the first
token — processing an 8k prompt on CPU takes minutes and that is not a hang —
and a tighter one between chunks, because silence after generation started
means a stuck server.

## Verification and proof of delivery

When the run finishes, a cheap check runs over what it claims to have done. No
model involved: do the files it wrote exist? did any command exit with an error?

The goal is not to audit the work — it is to catch the easy lie, the one small
models tell most: announcing a file that the tool never created. The trail shows
**Result verified** or **Verification found problems**.

That check on its own had a hole, and it was the worst one available: it returns
nothing when nothing was written and nothing was run — so the only run that
needed checking was exactly the one that escaped. A run that thought for a
minute, wrote a paragraph and closed as *Done* went straight through. The
guard-rails above missed it too: nearly all of them assume a tool call, and
whoever never acts trips none of them.

The answer was to move the proof **inside each delivery** instead of leaving it
to the end. The request is cut into steps before any work, and none of them
closes without passing three layers — acceptance command, files on disk, and,
only when the first two have nothing to look at, a judge of the written
criterion. The full rule, with what each layer may and may not decide, is in
[work modes and plans](/agent/plans).

Two consequences worth saying out loud. First: a step that only talks now
**counts as stalled** — before, a model that thought for three minutes without
acting tripped no guard-rail at all, because no tool had run. Second: when the
step budget runs out with work pending, the result names the deliveries left
over, instead of only saying *"stopped at the limit"*.

None of this replaces the immediate nudge for a model that announces work it did
not do — that one is lexical, cheap, and fires mid-loop. It only catches the
*promise* ("I'll create the three files"), never the false claim ("done, I
created the three files"). The three layers are the factual part underneath it,
and they act against the disk.

The plan board shows on every run, and it is what answers "which step is it on"
without anyone having to interpret *Thought for 60.6s* — with how long each
delivery took, and the estimate for the ones still ahead.

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

## The session terminal

The trail tells you *what* was run. The terminal tells you what it is printing,
while it prints: a panel in the chat where every command the agent executes
shows up live — the command line, the folder it runs in, the output arriving as
it happens, and the result at the end.

Before, output only surfaced once a command had finished, spread across the
trail's cards. A three-minute test suite stayed silent until the very end, and
there was no way to tell a slow build from a stuck one. That distinction is the
whole point of watching output stream.

Each command shows the line that ran, the folder it ran in, the output as it
arrives, the state (*Running*, *Done*, *Failed*), how long it took, and a notice
when the output was truncated. Reopening an old conversation brings the output
back from the database, so the panel doubles as a record.

The panel opens by itself the first time a command starts in a task; close it
and it stays closed until the next one. The button is at the top right of the
chat, next to the trail's, marked `>_`. The three right-hand panels — parameters,
trail, terminal — are exclusive: opening one closes the others.

Two things to expect, both consequences of not emulating a terminal: output
arrives without colours, and a progress bar shows up as repeated lines instead
of updating in place. That is not a display bug — it is exactly the text the
model reads. And this panel shows the *agent's* commands; llama-server's own
logs stay on the **Local Server** screen.

## Next

- [Authorization and permissions](/agent/authorization) — who decides what runs.
- [Tools](/agent/tools) — the catalogue, by family.
- [Work modes and plans](/agent/plans) — how big tasks get cut down.
- [Code Mode](/agent/code-mode) — one step, many calls.

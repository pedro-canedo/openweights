# Work modes and plans

A large task handed to a local model in one piece fills the window, the model
loses the thread, and it starts making things up. The answer the harness uses is
always the same: **cut the goal into small deliveries and run one at a time,
each with a fresh context.**

## The four work modes

| Mode | What it does |
|---|---|
| **Chat** | Answers only, no tools. |
| **Planning** | Investigates and proposes the plan. Changes nothing until you approve. |
| **Agent** | Runs the requested task, one step at a time. |
| **Loop** | Runs the whole plan, checking each delivery before moving on. |

Planning mode is the honest one for unfamiliar work: you see what it intends to
do — and the files it expects to touch — before anything is written.

## How a plan is built

The model is asked for the plan with a **forced JSON schema**. llama-server
turns the schema into a grammar, and that is what makes an 8B model return
something parseable at all.

The schema asks for the **files** each delivery will produce, and the prompt
explains what to put there. So does `plan_create`, the tool Planning mode uses
to register its plan — which means a plan you approve there reaches Loop mode
already saying what each step will create. That field is not decoration: it is
what makes the end of a run verifiable, and without it the delivery check has
nothing to look at.

Even so the result is validated with suspicion: a plan that does not survive
validation becomes a single-delivery plan. Decomposition never takes the run
down. Paths that could never be found under the project folder are dropped as
the plan is parsed — absolute paths, `..`, a Windows drive letter. Keeping one
would create a pending item that can never be satisfied, and it would be chased
forever.

Ceilings: **12 deliveries** per plan, **8 steps** per delivery, **2 attempts**
before a delivery is considered stuck. The run's global step budget still
applies on top.

## Deliveries in Agent mode

Cutting the request into deliveries is no longer exclusive to Loop mode. In
**Agent** mode the request is cut too, and the board shows there as well — so
"which step is it on" has an answer that is not *Thought for 60.6s*.

The reason is the delivery check: when the loop stops, the declared files are
looked for on disk, and while any are missing the run is told which ones and
carries on from where it stopped instead of closing as done.

It is deliberately **Agent mode only**. Loop mode does not need it — the plan
runner already re-queues a step that fails its check, and stacking both would
bill the same delivery twice. Planning mode must not have it: that mode ends
with the plan written and nothing done, which is exactly the state a chase reads
as unfinished work. See [how a run works](/agent/) for the full rule, including
the two cases the harness deliberately refuses to chase.

## What crosses between steps

Not the history — a **handoff** of up to three lines.

This is the whole trick. Because only a summary crosses, the window does not
grow with the size of the request; it grows with the size of *one* step. The
plan board shows `Context reset for this step` when that happens.

## The board

While a plan is running, the chat shows it: goal, deliveries with their status
(queued, in progress, done, blocked, failed, skipped), what each one depends on,
the files it expects to touch, and the *done when* condition.

In planning mode the board has **Approve plan** and **Redo plan** — nothing runs
before you approve. A delivery that gets stuck is marked blocked with a reason,
and the run moves on to what does not depend on it rather than dying.

## Window budget

The board also states the arithmetic it is working under: *model window: N
tokens — deliveries up to M*. Deliveries are sized against the window you
actually loaded, not against a hypothetical one.

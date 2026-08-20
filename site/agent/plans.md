# Work modes and plans

A large task handed to a local model in one piece fills the window, the model
loses the thread, and it starts making things up. The answer the harness uses is
always the same: **cut the goal into small deliveries and run one at a time,
each with a fresh context.**

## The three work modes

| Mode | What it does |
|---|---|
| **Chat** | Answers only, no tools. |
| **Plan** | Investigates and proposes the plan. Changes nothing until you approve. |
| **Execute** | Cuts the request up and runs delivery by delivery, proving each one. |

Plan mode is the honest one for unfamiliar work: you see what it intends to do —
and the files it expects to touch — before anything is written.

There used to be a fourth mode, **Loop**, doing what Execute does today. The
split did not hold up: Agent mode built a plan and then ignored it — running
loose and only checking at the end — so "with a plan" and "without a plan" were
two grades of run, and the worse one was the default. There is one path now.

## How a plan is built

The model is asked for the plan with a **forced JSON schema**. llama-server
turns the schema into a grammar, and that is what makes an 8B model return
something parseable at all.

The schema asks three things of every delivery: the **files** it will produce, an
**acceptance command** that exits 0 once it is done, and the *done when*
criterion in plain words. None of them is decoration — they are the three layers
of proof in the next section. `plan_create`, the tool Plan mode uses to register
its plan, asks for the same: a plan you approve there already says how each step
will be checked.

The schema also asks for **up to four questions**, for the decisions that change
the plan and that only you can make. When they show up, nothing runs: see
[questions](#questions-before-any-work).

Even so the result is validated with suspicion: a plan that does not survive
validation becomes a single-delivery plan. Decomposition never takes the run
down. Paths that could never be found under the project folder are dropped as
the plan is parsed — absolute paths, `..`, a Windows drive letter. Keeping one
would create a pending item that can never be satisfied, and it would be chased
forever.

Ceilings: **12 deliveries** per plan, **8 steps** per delivery, **2 attempts**
before a delivery is considered stuck. The run's global step budget still
applies on top.

## How a delivery proves it is done

"Done" stopped being the model's word. Every step goes through three layers, in
this order — and the first two are mechanical, with no model judging anything:

1. **The acceptance command.** It runs *before* the step (where it is expected
   to fail — that is the red test) and *after* it. A non-zero exit at the end
   fails the delivery. It goes through the same authorization policy as any
   command: turn the terminal family off and it does not run.
2. **The files.** What the step said it would write has to be on disk.
3. **The criterion judge.** It only steps in when the first two decided nothing
   — a step with no files and no checkable command — and it **never** overturns
   a mechanical failure: it breaks ties over emptiness, it does not veto disk.

A failed step goes back to the queue with the reason written down, and the next
attempt receives it. On the second failure it gets stuck, and the run moves on
to whatever does not depend on it. When the step budget runs out with work
pending, the result **names** the deliveries left over — running out of budget
is no excuse for not saying what is missing.

## Questions before any work

If cutting up the request runs into a decision that changes the plan — which
stack, how far the scope goes, where to save —, the run **stops before the first
step** and asks. Up to four questions, with clickable options when they are few
and known.

The pause is durable: the question is stored with the plan and survives closing
the app. It holds in every mode, including automatic runs and scheduled ones —
which have no conversation to answer in, so they show up in the **Waiting for an
answer** queue on the Activity screen, with a system notification.

Answering resumes the plan where it stopped. When the question came before any
work, the plan is rebuilt with the answer in view — it changed the plan, which
is why it was asked.

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

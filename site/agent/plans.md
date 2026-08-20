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

The planner never receives the whole spec: it receives an **outline** — the
phases at one line each, when the request already comes written that way, or the
first 2,500 characters. Sending the full text just to *split* it burns the window
at exactly the step that needs it most.

Even so the result is validated with suspicion — and what happens when it does
not survive has changed. Before, any failure became a single-delivery plan, and
a six-phase spec was run as if it were a short request. Now the fallback is the
**request itself**: a text already written as `Phase 1`, `Fase 2`, `Step 3`
becomes one delivery per phase, and a long request with no headings at all is
sliced into chunks. The single-delivery plan is left with what it should always
have been — a short request. Decomposition never takes the run down.

The same ruler applies to a plan that *did* pass validation with one giant
delivery. A small model facing a six-phase spec answers "implement the app", and
that is not a plan: it is the request handed back. When the request has phases
written in it and the plan has a single delivery, the phases win.

Paths that could never be found under the project folder are dropped as
the plan is parsed — absolute paths, `..`, a Windows drive letter. Keeping one
would create a pending item that can never be satisfied, and it would be chased
forever.

Ceilings: **12 deliveries** per plan, **8 steps** per delivery, **2 attempts**
before a delivery is considered stuck. The global step budget still applies on
top, but it no longer rules the *number of deliveries*: at 24 steps and 8 per
delivery no plan ever went past three, and a six-phase spec lost half of itself
before starting. The window sizes the deliveries; the step budget sizes the
work.

## How a delivery proves it is done

"Done" stopped being the model's word. Every step goes through three layers, in
this order — and the first two are mechanical, with no model judging anything:

1. **The acceptance command.** It runs *before* the step (where it is expected
   to fail — that is the red test) and *after* it. A non-zero exit at the end
   fails the delivery. It goes through the same authorization policy as any
   command: turn the terminal family off and it does not run. A command that
   would pass on any machine — `node -v`, `ls`, `pwd`, `echo` — is refused as
   proof while the plan is parsed: that was what kept the check green with an
   empty folder.
2. **The files.** What the step said it would write has to be on disk.
3. **The criterion judge.** It only steps in when the first two decided nothing
   — a step with no files and no checkable command — and it **never** overturns
   a mechanical failure: it breaks ties over emptiness, it does not veto disk.

Ahead of all three, for a step that promises code, comes a blunter requirement:
**some file written during this step**. A delivery that declares files, or whose
instruction asks for implementation, does not close with prose — thinking,
listing folders and running `node -v` is not a delivery, and it now fails in
those words.

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

The handoff lives in the window, and the window gets wiped. So the run also
writes **`.openweights/progress.md`** into the project folder: what already ran,
the files touched, the next step, the blockers. It is rewritten every step and
outlives the run — open it in the editor, commit it if you want. Do not confuse
it with [memory](/agent/memory): memory is a curated fact that holds forever,
progress is the scratch pad of *this* run.

## The rails of each phase

An 8B model does not fail out of ill will: it fails because nobody said, at that
step, what counts as done. Saying everything all the time does not work either —
a large system prompt pushes the request out of the window.

So the rails come in **per phase**, and only for the current one:

| Phase | Rails |
|---|---|
| Plan | split the spec into one delivery per phase, short instruction, an acceptance command that proves something |
| Execute | write a real file in this step, name what was created, check with a test or a build |
| Both | the window is short: the run's memory is `.openweights/progress.md`, not the history |

They ship inside the binary — a small model cannot depend on you remembering to
attach a guide. The whole section has a character ceiling, because a long rail
costs exactly what the request costs.

To replace a rail in this project, write `.openweights/skills/<name>/SKILL.md`
with the same `name` as the built-in one — `planning`, `implementation`,
`verification` or `context`. A file with an unknown name is ignored on purpose:
the window is far too small to accept whatever text shows up in the folder. In
Chat mode no rail is loaded at all.

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

Summarising the history near the ceiling is not enough when **one** message is
the problem. A whole spec pasted in can take half the window by itself, and then
there is no middle left to summarise: compaction runs and still nothing fits. So
a message past a fifth of the window is clipped before that — head and tail stay,
the middle becomes one line saying how many characters were dropped and where the
live cut is (`.openweights/progress.md`).

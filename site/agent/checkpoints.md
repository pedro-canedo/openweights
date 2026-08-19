# Checkpoints

Before the agent changes anything, the harness takes a photo of the project so
you can go back with one click.

## When one is taken

Before the **first change of a run** — the first `fs_write`, `fs_edit` or
anything else that touches disk. Reading never triggers one. In automatic
(YOLO) mode this is not optional: the checkpoint is part of what makes that mode
defensible.

The run trail shows *Checkpoint created* inline, with the time and which backend
took it. The **Checkpoints** list sits in the file explorer column, next to the
files it protects.

## Two backends

| Backend | How it works |
|---|---|
| **Shadow git** | A **parallel** git repository living in the app's data folder, versioning the project folder without ever touching your own `.git` — no renaming, no moving, no commits in your repository. It works by pointing `GIT_DIR`/`GIT_WORK_TREE` elsewhere. |
| **File copy** | A copy of the files about to change. Always available (no git required) and cheap when the run touches few files. |

The choice is automatic: git when it exists and the project is not enormous,
copy otherwise. The list tells you which one was used.

::: tip Your repository is never touched
This is the part worth repeating. A checkpoint does not commit, stage, stash or
rewrite anything in your git history. If you have uncommitted work when the
agent starts, it stays exactly as it was.
:::

## Restoring

Click **Restore** on any checkpoint. It asks first — *"Restore files to this
point? Later changes will be lost."* — because it is the one action in the
agent flow that throws work away.

Restoring rolls the *files* back. The conversation stays: you keep the record of
what was tried, which is usually what you want to write the next instruction.

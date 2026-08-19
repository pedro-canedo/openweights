# Authorization and permissions

Nothing the agent does escapes this page. Every tool call goes through the same
decision, and the decision is yours to configure.

## The four levels

Set in the composer, per conversation:

| Level | What it means |
|---|---|
| **No tools** | The model only talks. It runs nothing. |
| **Always ask** | Every tool asks for your confirmation. |
| **Ask for changes** | Reading runs on its own; writes and commands ask first. |
| **Automatic (YOLO)** | Runs on its own inside the project folder. A checkpoint is created before the first change. |

Automatic mode needs a project folder chosen, asks for an explicit confirmation
the first time, and stays scoped: **actions outside the folder, network access
and commands we cannot parse still ask.**

## How a decision is made

The policy runs in this order of precedence, and the first rule that matches
wins:

1. A **`never`** you set on a tool always wins.
2. **Outside the project folder** — always asks.
3. **A command we could not fully parse** — always asks. The confirmation says
   so instead of pretending it understood.
4. **Network access** — always asks, unless you marked that specific tool as
   *always allow*.
5. Only then the run's authorization level and the read shortcuts apply.

The point is to protect you from a *model* error — deleting the wrong folder,
running a script nobody read — while keeping the flow pleasant when the action
is obviously harmless.

## The confirmation

When a call needs you, the composer is replaced by the approval bar: the tool,
the arguments (expandable), the folder a command would run in, and warnings when
the target is outside the project or the command could not be parsed.

You can **Allow**, **Always allow** (in this project, or in every project),
**Deny**, or **Deny and explain** — the last one hands the model your reason, so
it tries something else instead of trying the same thing again.

Keyboard: <kbd>Enter</kbd> allows, <kbd>Esc</kbd> denies.

## Permissions per tool

**Settings → Tools → Permissions** sets a standing policy per tool: *Always
allow*, *Ask*, or *Never*. This is what survives across conversations, and it is
where a `never` — the rule nothing overrides — is set.

## Tool families

The same screen turns whole **families** on and off: Files, Terminal, Code, Git,
Data, Web, Memory, Project index, Planning, Connectors, Computer. Turning a
family off puts those tools out of the agent's reach entirely.

At least one family stays on. To stop the agent from using tools at all, the
honest control is the authorization level — *No tools* — not an empty menu.

::: info The window still has the last word
Even with every family on, the agent only receives the tools that fit the
model's context window. The rest stays reachable — it asks for them by name when
it needs them. See [Tools](/agent/tools).
:::

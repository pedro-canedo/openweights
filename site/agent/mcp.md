# MCP connectors

[Model Context Protocol](https://modelcontextprotocol.io/) servers become tools
the agent can use — through the same confirmations as the native ones.

## Adding one

**Settings → Connectors** takes either form:

- **Fill in manually** — name, type, and the details for it.
- **Paste JSON** — the `{"mcpServers": { ... }}` block you already have from
  another client works as-is.

Two transports:

| Type | What it is |
|---|---|
| **Local program** (stdio) | A command on your machine, with arguments and environment variables |
| **Remote server** (HTTP) | A URL, with headers |

**Test connection** connects and reads the catalogue. You have to test before
the tools can be reviewed — the app will not list tools it has not actually
seen.

## The approval gate

An MCP server announces its tools at runtime, and it can change them after you
approved it. That is the *rug pull*, and the harness closes it:

Every time the catalogue is listed, its hash is compared with the one you
approved. While the two differ, **the server exposes no tools at all** — not to
the model (it never reaches the prompt) and not to execution (calls are refused
with an explanation). The connector is marked *Awaiting review* until you look
at what changed and approve again.

Closing both paths matters. Blocking only execution would leave the model
insisting on a phantom tool.

## In the tool catalogue

Each server becomes a tool provider, and names are prefixed with the server id —
`github__create_issue`. That is what puts MCP tools through the same policy, the
same events and the same approval bar as native ones. There is no parallel path.

Tools carry the badges the server declares: **read only**, **destructive**,
**reaches the internet**. Connectors are their own [tool
family](/agent/tools#the-families), so turning the family off retires all of
them at once.

When a connector needs information mid-run — an *elicitation* — the run pauses
and asks you, in the trail, instead of guessing.

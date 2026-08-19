# Tools

The catalogue is past thirty tools. An 8B model with an 8k window would spend a
large slice of it just reading descriptions — and, worse, picks *worse* the more
options it sees.

So the harness serves a menu, not the whole catalogue.

## How the menu is built

Like a restaurant: show few dishes, keep a waiter for the rest.

1. **The core goes in first** — `fs_read`, `fs_list`, `fs_grep`, `fs_edit`,
   `fs_write`, `terminal_run`. In a tiny window `fs_read` is the last to go.
2. **Then what the objective asked for** — the request itself is the best clue
   about which tools matter now.
3. **Up to the ceiling the window can take.**
4. **The waiter** — `tools_find` — lets the model ask for anything left out, by
   name or by what it needs to do. What the model activated itself never gets
   taken away behind its back.

In loop mode the menu is re-evaluated at every step of the plan: the current
step's instruction is the freshest clue available.

The whole thing is deterministic and testable without a network: same input,
same menu.

## The families

| Family | What it covers |
|---|---|
| **Files** | Read, create, edit and find files in the project folder |
| **Terminal** | Run commands on your computer |
| **Code** | Detect the project, build, run tests, lint and format |
| **Git** | History, diffs, staging, commits, branches, stash, restore |
| **Data** | Preview and query CSV files and SQLite databases |
| **Web** | Search, open pages and download files from the internet |
| **Memory** | Keep what should be remembered in later chats |
| **Project index** | Search indexed project files by meaning |
| **Planning** | Split the task into steps, and ask you when unsure |
| **Connectors** | Tools lent by the [MCP connectors](/agent/mcp) you turned on |
| **Computer** | Clipboard, system notifications, opening files or links |

## The catalogue

| Tool | What it does |
|---|---|
| `fs_read`, `fs_write`, `fs_edit`, `fs_append` | Read a file, create/replace one, edit one, append to one |
| `fs_list`, `fs_glob`, `fs_grep` | List a folder, find files, search contents |
| `terminal_run` | Run a command |
| `project_info` | Detect what kind of project this is |
| `build_run`, `test_run`, `lint_run`, `format_run` | Build, test, lint, format |
| `code_run` | Run a script |
| `git_status`, `git_diff`, `git_log` | See the state and the history |
| `git_add`, `git_commit`, `git_branch`, `git_stash`, `git_restore` | Change it |
| `csv_preview`, `csv_query`, `data_summary` | CSV work |
| `sql_query`, `sql_schema` | SQLite work |
| `web_search`, `web_fetch`, `web_download`, `http_request` | The internet |
| `workspace_search` | Search the project index by meaning |
| `memory_save` | Remember a fact |
| `plan_create`, `plan_update`, `task_complete`, `todo_update` | Split into steps, update one, finish one, keep the plan current |
| `ask_user` | Ask you, when guessing would be worse |
| `agent_delegate` | Hand an investigation to a helper with a fresh context |
| `tools_find` | Ask for a tool that is not on the current menu |
| `clipboard_read`, `clipboard_write`, `notify_user`, `open_path` | The computer around the app |
| `run_code` | Write a program that uses the tools all at once — [Code Mode](/agent/code-mode) |

Every one of them goes through [authorization](/agent/authorization) — including
the calls a Code Mode program makes.

## Web search quality

Without a key the agent searches through DuckDuckGo: free, lower quality, and
prone to blocking. Adding a **Brave** or **Tavily** key in Settings makes
results noticeably better. On automatic, the key shape picks the provider —
`tvly-…` means Tavily, anything else means Brave.

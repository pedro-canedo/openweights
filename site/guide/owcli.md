# OwCLI: agent and terminals

**OwCLI** is OpenWeights' terminal screen, right below AgenticOw in the
sidebar. Every session there is a real terminal: the OwCLI agent, a plain
shell (bash, zsh, PowerShell) or an outside agent such as Claude Code. Several
stay open at once, side by side, and keep running when you switch screens.

The OwCLI agent is OpenWeights' fork of
[OpenAI's Codex CLI](https://github.com/openai/codex) (Apache-2.0 license): the
upstream core, with the branding, privacy and your models on top. Its
interface is in English for now.

## The screen

Open sessions are on the left. Each shows the title the program gave its
terminal, the folder it is in and a status dot: green while it runs, dim once
it ended (with the exit code), amber when it needs you. The arrow keys move
through the list.

The grid is on the right. The **Layout** buttons at the top show 1, 2 or 4
panes, and the divider between them drags with the mouse or the arrow keys.
The choice is remembered. Clicking a session in the list puts it in the
focused pane. A session shows in one pane only, and shrinking the grid hides
panes without closing anything.

The **+** at the top of the list (**New session**) opens both ways in: **OwCLI
agent…** or **New terminal**. With the screen empty, the same two buttons sit
in the middle of it, and every empty pane of the grid offers both too.

Switching screens stops nothing: the terminal keeps running and drawing, and
what it wrote is there when you come back. Closing a session (the **×** in the
list or the right-click menu) ends the program and everything it started.

## The agent

**New session → OwCLI agent…** (or **Open the OwCLI agent**, on the empty
screen, or the same command in the palette, Ctrl+K) asks four things before
opening:

- **Model**: the models OpenWeights can serve right now, grouped by source.
  The choice is remembered for the next session.
- **Working folder**: your home folder, or the one you pick.
- **When to ask for your approval**: *when it needs to* (recommended), *before
  any command* or *never*.
- **What it can do**: *read and edit the folder* (recommended), *read only* or
  *everything, no limits*.

Inside, `/model` switches models in the same conversation, across every
source.

### Installing the agent

The agent does not ship inside the app: it is a separate program, downloaded
the first time you pick it. Before downloading, the app shows the size on the
button itself, and only downloads if you accept. The package is checked
against the size and sha256 the app carries, and tried on this machine before
it stays: a package that does not run here is not kept, and **Try again**
downloads it again. It lives in the app's folder (`runtimes/owcli`). Closing
the window during the download does not stop it, and once installed, the app
goes straight to the open dialog.

There are packages for Linux x64, Windows x64 and macOS. On a computer with no
package, the app says the agent is not available there, and the terminals work
as usual.

### Where the brain comes from

OwCLI has no provider, key or login of its own: it thinks only with the app's
models. Those are the Local Server's, your OpenRouter favorites and 9router's,
the same ones AgenticOw uses. When there are none (the server is stopped and
no remote source is on), the dialog turns into **Choose OwCLI's brain** and
shows the next step for each source. **Start the Local Server** brings the
engine up right there, and the dialog comes back with the models.

**You see the model thinking.** Local models think before they answer, sometimes
for minutes, and the agent used to show only "Working". Sessions opened by the
app now show the model's reasoning as it writes it, so you can follow what it is
weighing and where it is heading. The [logs panel](/integrations/local-api#seeing-what-is-running)
(`Ctrl+Shift+L`) shows the engine's own side: tokens generated and speed.

## History

Below the open sessions are the conversations OwCLI saved. Each shows its name
(or the start of the first message), when it was and in which folder. Clicking
continues the conversation in a new session, in its folder and with its model,
if that model is still available. The pencil gives the conversation a name.

## When it needs you

When the agent asks approval for a command and you are not looking at that
session, the session gets the amber dot and the **needs your attention** note
in the list. With the app window unfocused, a system notification arrives too, at
most one every 15 seconds per session. Opening the session clears the note.

## Copy and paste

**Ctrl+Shift+C** copies the selection and **Ctrl+Shift+V** pastes; the
right-click menu does the same and also clears the screen. Programs that copy
through the terminal (the OSC 52 sequence, used by tmux and Neovim) reach the
system clipboard.

## Other agents in a session

Claude Code, Aider and OpenCode open here too, from **Local Server → Use it in
another app**: the app starts the program in a session on this screen, pointed
at your local API and with the chosen model. On Windows, the card also offers
**External terminal**, which opens the program in a window of its own.

## The `owcli` command in the system terminal

In **Settings → OwCLI in the system terminal** (Advanced mode), **Turn on the
owcli command** makes the agent available in any terminal, with the same models as
the OwCLI screen and the same home (`~/.owcli`), where conversations are kept. Nothing changes in your terminal
without that click, and **Turn off** undoes only what the app did.

- **Linux and macOS:** a link at `~/.local/bin/owcli` to the installed version.
  If a file with that name already exists and is not OpenWeights', the app says
  so and leaves it alone. On macOS, and on some Linux setups, `~/.local/bin` is
  not on the PATH: the card shows the line to put in `~/.zshrc` or `~/.bashrc`.
- **Windows:** the agent's folder is added to your user `Path` (the machine's is
  not touched). Open a new terminal after turning it on.

When the agent is updated, the command points to the new version, and the old
one is only deleted when no session runs from it. Since the agent thinks with
OpenWeights' models, **the app has to be open** when you use the command; when
it is closed, `owcli` says so and exits.

## Privacy

- OwCLI talks only to OpenWeights, on `127.0.0.1`, and proves who it is with a
  token from the app. The OpenRouter and 9router keys never leave the app:
  OpenWeights itself adds them to each request.
- Telemetry, feedback, version checks and the features that talk to OpenAI
  services are off.
- OwCLI's conversations and settings live in its own home (`~/.owcli`),
  separate from any Codex installed on the same machine.

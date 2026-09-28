# OwCLI: terminals

**OwCLI** is OpenWeights' terminal screen, right below AgenticOw in the
sidebar. Every session there is a real terminal: a plain shell (bash, zsh,
PowerShell) or an outside coding agent such as Claude Code. Several stay open
at once, side by side, and keep running when you switch screens.

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

Switching screens stops nothing: the terminal keeps running and drawing, and
what it wrote is there when you come back. Closing a session (the **×** in the
list or the right-click menu) ends the program and everything it started.

## When a program needs you

When a program asks for your attention (an agent asking approval for a
command, for instance) and you are not looking at that session, the session
gets the amber dot and the **needs your attention** note in the list. With the
app window unfocused, a system notification arrives too, at most one every 15
seconds per session. Opening the session clears the note.

## Copy and paste

**Ctrl+Shift+C** copies the selection and **Ctrl+Shift+V** pastes; the
right-click menu does the same and also clears the screen. Programs that copy
through the terminal (the OSC 52 sequence, used by tmux and Neovim) reach the
system clipboard.

## Coding agents in a session

Claude Code, Aider and OpenCode open here too, from **Local Server → Use it in
another app**: the app starts the program in a session on this screen, pointed
at your local API and with the chosen model. On Windows, the card also offers
**External terminal**, which opens the program in a window of its own.

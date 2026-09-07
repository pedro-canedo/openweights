# Security policy

## Reporting a vulnerability

Report privately through
[GitHub's advisory form](https://github.com/pedro-canedo/openweights/security/advisories/new),
not as a public issue. Include what you did, what happened, and the version and
system — the same three things the bug template asks for, because a security
report is a bug report that should not be public yet.

This is a small open source project with no security team and no bounty. Expect
a first reply within a week. If the report is valid, the fix ships in the next
release and the advisory is published with it.

## Which versions get fixes

The latest release. The project is at `0.x` and moves fast: there are no
maintained branches for older versions, and the answer to "is this fixed in
0.12?" is "update".

## Known limitations, by design

These are documented decisions, not vulnerabilities. Reporting them is welcome
as an improvement proposal, but they will not be treated as an embargoed issue.

**API keys are stored in plain text.** Keys you paste into the app (OpenRouter,
Hugging Face, and any provider) live in the local SQLite database alongside the
rest of the settings. There is no OS keyring integration yet. The file never
leaves your machine, but any program running as your user can read it.

**The binaries are not signed by a certificate authority.** There is no paid
code-signing certificate, so Windows shows SmartScreen and macOS shows the
Gatekeeper warning. The macOS bundle carries an *ad-hoc* signature, made by the
machine that compiled it: it is not provenance, and it is not claimed to be.
Verify downloads come from the
[releases page](https://github.com/pedro-canedo/openweights/releases) of this
repository. What *is* verified is the updater: update packages are signed with
a minisign key and an installed app refuses a package that does not match.

**The local API server has no authentication by default.** It listens on
`127.0.0.1`, which means the machine itself. Turning on local-network access
removes that boundary and anyone on the same network reaches your models — the
app says so at the moment you turn it on, and an API key is available. Only
enable it on a network you trust.

**The RPC cluster has no password.** Lending a GPU over the network opens a
control channel on the local network, off by default on both machines, and
turning it on is the consent. It is meant for a network you own. The pairing
handshake refuses an unsolicited acceptance and cannot be used to displace a
live pair, but it is not a substitute for a trusted network.

**Models run code you asked for.** The coding agent reads and edits files and
runs commands. That is its purpose, and the approval flow exists so that it is
your decision each time. A model that convinces you to approve something
harmful is a prompt-injection risk inherent to agents, not a flaw in this app —
treat untrusted content in a repository the way you would treat an untrusted
script.

## What is in scope

Anything that lets someone reach your data or your machine without you
enabling it: a path traversal in file handling, a way to bypass the updater
signature, a request that escapes the approval flow, a secret written somewhere
it should not be, or the local server answering beyond the boundary it
declares.

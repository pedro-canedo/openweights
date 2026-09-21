# Troubleshooting

Most problems here come from one of four places: the operating system not
trusting an unsigned binary, the engine not matching the machine, a model not
fitting the card, or something else already using the port. Each section below
says what you see, why, and what fixes it.

If none of this covers it, the log in **Local Server → Advanced** and an
[issue](https://github.com/pedro-canedo/openweights/issues) are the next step —
the issue template asks for hardware and model because almost nothing here can
be diagnosed without them.

## The system refuses to open the app

The binaries carry no paid code-signing certificate, so both systems warn you.
This is expected, and the warning is about provenance, not about the app doing
anything.

| System | What you see | What to do |
|---|---|---|
| Windows | *Windows protected your PC* | **More info** → **Run anyway** |
| macOS 15+ | *cannot be opened* | Try once, then **System Settings → Privacy & Security → Open Anyway** |
| macOS 14 and older | *cannot be opened* | Right-click the app → **Open** |

From the macOS terminal, this settles it in one line:

```bash
xattr -dr com.apple.quarantine /Applications/OpenWeights.app
```

The installer script does this for you; the warning appears when you download
the `.dmg` by hand.

## The engine card says something is wrong

The app runs its own llama.cpp and asks the binary which build it is, rather
than trusting the folder name. The verdict is one of five:

| Verdict | What it means | What to do |
|---|---|---|
| **Ready** | It ran and answered with the build this release expects | Nothing |
| **Not installed** | First run, or the folder was deleted | Install it from **Settings → AI engine** |
| **Update available** | The disk has a different build than this release was tested with | Update it |
| **Wrong variant** | Your GPU or driver changed since the install | Reinstall — the app picks the variant again |
| **Won't run** | The files are there and the executable does not start | Reinstall. A CUDA package missing its runtime DLLs passes every file check and only fails here |

## The engine download fails or stalls

It is a few hundred megabytes, fetched from a pinned GitHub release. A
corporate proxy or an antivirus scanning the archive are the usual causes — on
a machine with real-time scanning, expect it to take several minutes rather
than seconds. The install is atomic: a failed download leaves the previous
engine working.

## No GPU is detected

The status bar shows CPU only, and models grade worse than you expect.

- **NVIDIA**: the driver must be recent enough for the CUDA build the app
  picked. A driver that predates it makes the engine fall back or refuse.
- **Integrated graphics** may not be reported at all. This is a detection gap,
  not a fault — the app uses the discrete card when there is one.
- **After changing a card or driver**, reinstall the engine: the variant is
  chosen at install time, and the card underneath changed.

## A model will not load

The verdict colour in **Discover** and **My Models** is an estimate of whether
the file fits your card, made before you download. When a model that graded
green refuses to load anyway:

- **Context size is part of the arithmetic.** A model that fits at 4K may not
  fit at 32K — the KV cache grows with the window. Set an explicit context
  instead of asking for the maximum.
- **Concurrent models multiplies it.** With the setting above 1, models stay
  loaded together and share the same video memory.
- **Something else is using the card.** A browser with hardware acceleration, a
  game, or another OpenWeights instance all take their slice.

Lowering GPU layers moves part of the model to system RAM: slower, but it runs.
See the [configuration reference](/integrations/configuration).

## A Bonsai model says it needs the PrismML engine

`PTQ1_0` and `PQ2_0` files only load on PrismML's fork of llama.cpp. The app
installs that engine together with the download; if it is missing anyway, the
chat and the model card show an **Install PrismML engine** button — about
150 MB from a pinned GitHub release, verified by running the binary. The
engine switches automatically when you pick the model and switches back on the
next one; the Local Server header says which one is running. Optimization and
the performance tools measure a Bonsai model **on that engine**, since a
measurement is just the file being opened by a binary. The one thing they skip
is the MoE-cache arm, which is a different fork and cannot open these files;
the network GPU (cluster) also stays on the official build.

## The local server will not start

| What you see | Usually means |
|---|---|
| *port already in use* | Another process holds it — change the port in **Network**, or stop the other process |
| The server starts and stops immediately | The engine failed to load a model. The reason is in **Advanced** → the log |
| *the engine is not installed* | Install it in **Settings → AI engine** first |

A setting that applies at start time cannot change while the server runs. The
screen says so, and stopping and starting is part of applying it.

## An external app cannot reach the API

- **The address is `127.0.0.1` by default**, which means this machine only.
  Reaching it from another device requires turning on local-network access in
  **Network**.
- **A key set means a key required.** The request must send
  `Authorization: Bearer <key>`.
- **The `model` field must match** an id from `GET /v1/models`.

The [API reference](/integrations/api-reference) lists the endpoints and what
each error status means.

## Extra GPU on the network

| What you see | What it means |
|---|---|
| *No other OpenWeights visible on this network* | The other machine has the feature off, is on another network, or mDNS is blocked by the firewall |
| *different tag — update* | The two apps ship different llama.cpp builds. Update both |
| *The installed engine does not include the RPC worker* | The engine is from an older package. Update it in **Settings → AI engine** |
| *the local server is running on this machine* | Stop the local server on the helper before lending its GPU |

## The coding agent will not install

The first open downloads a portable Node runtime and its packages — hundreds of
megabytes, and several minutes on a machine with antivirus scanning every file.
The live log is on the harness screen. It installs into a folder of its own and
never touches a Node you have installed.

## Answers are slower than they should be

Before changing settings, find out which phase is slow: **Run details** after a
response separates queueing, model loading, waiting for the first token and
generation. A slow first token on every message is usually a model being
reloaded; slow generation throughout is a configuration question.

[Chat performance](/guide/performance) explains how to compare two
configurations honestly, with warmup and repetitions, instead of trusting a
single run.

## Reporting something else

Open an [issue](https://github.com/pedro-canedo/openweights/issues) with your
hardware, the model, and the log from **Local Server → Advanced**. Nearly every
question here depends on those three, and without them investigating turns into
guesswork.

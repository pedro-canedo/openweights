# Install

## One line

::: code-group

```powershell [Windows]
irm https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.ps1 | iex
```

```bash [macOS and Linux]
curl -fsSL https://raw.githubusercontent.com/pedro-canedo/openweights/main/scripts/install.sh | sh
```

:::

The script picks the right file for your system, installs it, and on macOS it
also clears the quarantine flag so the app opens without a fight.

## By hand

Grab the file for your system from the
[latest release](https://github.com/pedro-canedo/openweights/releases/latest):

| System | File |
|---|---|
| Windows 10/11 (x64) | `OpenWeights_x.y.z_x64-setup.exe` |
| macOS 11+ (Apple Silicon and Intel) | `OpenWeights_x.y.z_universal.dmg` |
| Linux x64 (Debian/Ubuntu) | `OpenWeights_x.y.z_amd64.deb` |
| Linux x64 (any distro) | `OpenWeights_x.y.z_amd64.AppImage` |

Once installed, the app **checks for new versions on its own** and offers a
one-click update — you do not need to come back here.

## The unsigned-binary warning

**The binaries are not signed.** Code signing requires a paid, yearly
certificate the project does not have. Your system will warn you; saying so
plainly is better than pretending the warning is a bug.

**Windows** — on *"Windows protected your PC"*: click **More info** → **Run
anyway**.

**macOS** — the one-line install above already clears it. If you downloaded the
`.dmg` by hand and got *"Apple could not verify this app is free of malware"*:

- **macOS 15 (Sequoia) or newer**: try to open it once, then go to *System
  Settings → Privacy & Security*, scroll to the notice about OpenWeights and
  click **Open Anyway**.
- **macOS 14 or older**: right-click the app → **Open**.
- **Any version**, from the Terminal:

  ```bash
  xattr -dr com.apple.quarantine /Applications/OpenWeights.app
  ```

What the macOS build *does* have is an **ad-hoc signature**, made by the machine
that compiled it. It is not proof of origin — it only keeps the system from
refusing a universal `.dmg` outright.

## Disk space

The installer is small on purpose: no GPU stack ships inside it. On first launch
the app downloads the llama.cpp runtime that matches your card — **a few hundred
MB**, once. Models are downloaded separately and are the bulk of the disk use:
plan for a few GB per model.

## Uninstall

| System | How |
|---|---|
| Windows | *Settings → Apps → OpenWeights → Uninstall* |
| macOS | Drag `OpenWeights.app` to the Trash |
| Linux (`.deb`) | `sudo apt remove openweights` |
| Linux (AppImage) | Delete the file |

Models and conversations live outside the app bundle and survive uninstalling —
delete the data folder by hand if you want them gone.

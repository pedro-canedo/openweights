## Installation

Installers are attached for Windows x64, macOS universal and Linux x64.
Existing installations receive the signed updater metadata.

The binaries have no paid code-signing certificate, so your system will warn
you:

- **Windows**: on "Windows protected your PC" → *More info* → *Run anyway*.
- **macOS**: on macOS 15+, try to open it once and then allow it in
  *System Settings → Privacy & Security → Open Anyway*; on macOS 14 or older,
  right-click the app → *Open*. From the Terminal, this settles it:
  `xattr -dr com.apple.quarantine /Applications/OpenWeights.app`.
  The macOS bundle is ad-hoc signed.

On first launch, OpenWeights downloads the llama.cpp runtime that matches your
GPU (a few hundred MB) — that is why the installer is small.

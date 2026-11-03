# Hex typing fork

Based on [Hex 3.1.2](https://github.com/anomalyco/hex/releases/tag/v3.1.2),
commit `826bdbc44c8806630a45c08ed8cef35a36817a17`.

Primary and raw macOS dictation use synthesized keyboard events with the
active layout's key codes and Shift flags. They do not write the transcript to the clipboard or send
Command-V. Text enters after the recording ends. Live text in the HUD is
still a preview. Line breaks use Shift-Return to prevent chat submission.
The key-down and key-up each get a 1 ms pause. This follows Superwhisper
3.0.1's simulated-keypress loop, inspected locally with REA. The layout map
refreshes on the main thread when the input source changes. Dead keys and
characters outside the plain and Shift layers use Unicode events. Combining
sequences and emoji stay together when they fit Quartz's 20 UTF-16 unit limit.
The Chromium clipboard workaround keeps its existing fast event batches.

Copy-only mode, Paste Last, and meeting delta keep their previous behavior.
Shortcuts, models, text processing, meetings, and other saved settings
remain in the user's Application Support folder. Set the dictation shortcut
to `fn` in Hex to use the Globe key.

Build with Rust 1.97.0 and Xcode. Set `HEX_LOCAL_BUILD=1` so the updater
cannot replace the typing change. Set `CARGO_PROFILE_RELEASE_STRIP=none`
to avoid a misaligned LINKEDIT error when loading stripped Rust procedural
macros on macOS. The local app has an ad-hoc signature. macOS may require
new Microphone and Accessibility grants.

```sh
RUSTUP_TOOLCHAIN=1.97.0 CARGO_PROFILE_RELEASE_STRIP=none \
scripts/check -E 'test(keyboard::tests) or test(primary_dictation) or binary(keyboard_layout)'
HEX_RELEASE_UNSIGNED=1 HEX_LOCAL_BUILD=1 \
RUSTUP_TOOLCHAIN=1.97.0 CARGO_PROFILE_RELEASE_STRIP=none \
./scripts/build-app.sh
```

The source change and its tests are public. Runtime settings, account data,
recordings, and transcripts stay on the user's Mac. This fork does not
include signed release downloads; the download links in the upstream
sections of the README install stock Hex.

This fork also includes a local [Superwhisper importer](SUPERWHISPER_IMPORT.md)
and instructions for enabling Hex's existing cloud rewrite support through
OpenCode. These do not change the keyboard output path.

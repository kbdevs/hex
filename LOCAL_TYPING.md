# Hex typing fork

Based on [Hex 3.1.2](https://github.com/anomalyco/hex/releases/tag/v3.1.2),
commit `826bdbc44c8806630a45c08ed8cef35a36817a17`.

Primary and raw macOS dictation use the existing synthesized Unicode
keyboard events. They do not write the transcript to the clipboard or send
Command-V. Text enters after the recording ends. Live text in the HUD is
still a preview. Line breaks use Shift-Return to prevent chat submission.
Primary and raw dictation send one Unicode character at a time with 15 ms
between complete keystrokes. The first character starts without an added
wait. Emoji surrogate pairs stay together, and Shift-Return completes
before the next pause. This is a steady pace, without random human pauses.
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
cargo +1.97.0 test --locked --bin hex primary_dictation
cargo +1.97.0 test --locked --bin hex keyboard::tests
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

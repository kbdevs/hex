# Hex typing fork

The `main` branch includes our changes and upstream Hex 3.4.0 at
commit `ba8b5698ace26631d0ca0e8fe5caea48d5f91a3c`.

On macOS, Settings > Labs > Type dictation selects actual keyboard typing
for primary and raw dictation. Turn it off to use clipboard paste and restore.
The switch is off on a new install.
It applies to the next output without a restart. Copy dictation only takes
priority over both methods. Paste Last and meeting deltas use their existing
paste modes, including the Chromium 155/156 typing workaround.

Typing uses the active layout's key codes and Shift flags. Shift also sends
balanced modifier events so remote desktop apps can read it. It does not write
the transcript to the clipboard or send Command-V. Text enters after the
recording ends. Live text in the HUD is still a preview. Line breaks use
Shift-Return to prevent chat submission.
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

The installed executable is not on PATH. To change the output method in a
terminal, use:

```sh
/Applications/Hex.app/Contents/MacOS/hex labs on type-dictation   # type
/Applications/Hex.app/Contents/MacOS/hex labs off type-dictation  # paste
```

Build with Rust 1.97.0 and Xcode. Set `HEX_LOCAL_BUILD=1` so the updater
cannot replace the typing change. Set `CARGO_PROFILE_RELEASE_STRIP=none`
to avoid a misaligned LINKEDIT error when loading stripped Rust procedural
macros on macOS.

For local builds, save one signing certificate in this checkout's local Git
config. Use its SHA-1 from `security find-identity -v -p codesigning` and its
team ID from the certificate's subject `OU` field. An Apple Development
certificate's common-name suffix can differ from its team ID.

```sh
git config --local hex.localSigningIdentity CERTIFICATE_SHA1
git config --local hex.localSigningTeam TEAM_ID
```

These machine-specific values stay outside Git history. With a profile saved,
local builds require that certificate and reject unsigned builds or conflicting
signing overrides. Test builds and release signing keep their existing rules.
Keep the bundle ID `anomaly.hex` and the same certificate so macOS can retain
permission grants across rebuilds. Moving from an ad-hoc signature to a
certificate may require one final grant. macOS can also reset grants if the
identity or its security requirements change.

`sh scripts/test-local-signing.sh` checks profile selection and rejection
without building an app or using a private key.

```sh
PATH="$HOME/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH" \
CARGO_PROFILE_RELEASE_STRIP=none \
scripts/check -E 'test(keyboard::tests) or test(primary_dictation) or binary(keyboard_layout)'
PATH="$HOME/.rustup/toolchains/1.97.0-aarch64-apple-darwin/bin:$PATH" \
HEX_LOCAL_BUILD=1 CARGO_PROFILE_RELEASE_STRIP=none bash scripts/build-app.sh
```

The source change and its tests are public. Runtime settings, account data,
recordings, and transcripts stay on the user's Mac. This fork does not
include signed release downloads; the download links in the upstream
sections of the README install stock Hex.

This fork also includes a local [Superwhisper importer](SUPERWHISPER_IMPORT.md)
and instructions for enabling Hex's existing cloud rewrite support through
OpenCode. These do not change the keyboard output path.

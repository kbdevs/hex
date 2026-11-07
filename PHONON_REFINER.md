# Local refiner (Phonon)

Settings > Labs > Local refiner (Phonon) adds an optional local text cleanup
stage on Apple silicon Macs. It is off by default. It uses Hex's current speech
model and the text refiner from a separately installed Phonon app.

Install the signed Phonon app, then prepare only its text refiner from this
checkout:

```sh
brew install --cask infatoshi/tap/phonon
bash scripts/setup-phonon-refiner.sh
```

Setup downloads the pinned Phonon refiner (about 2.4 GB) and Python packages.
It loads the refiner with a fixed test phrase, then exits. It does not open
Phonon's UI, register a shortcut, use the microphone, download another speech
model, or read Phonon's history or preferences.

Turn on **Local refiner (Phonon)** in Hex Settings > Labs. The CLI can also
change it while Hex runs:

```sh
/Applications/Hex.app/Contents/MacOS/hex settings set labs.phonon_refiner true
```

Hex sends the corrected final transcript, up to 80 Hex vocabulary terms, and
an app category to the local text sidecar. Known chat apps use casual style;
work chat and email use formal style; editors and AI apps use prompt style.
Unknown apps use minimal cleanup. While this switch is on, Hex uses its
existing foreground browser reader to select the style for a website. Full
URLs and window titles are not sent to the sidecar.

The local stage takes the place of OpenCode rewriting while this switch is
on. It keeps the saved OpenCode settings. Transformations still run after
refinement. Dictate without processing, meetings, and Paste Last retain their
existing paths. Typed output and clipboard output work as before.

Normal dictation starts the sidecar with offline-only runtime and model
flags. A missing dependency, invalid reply, timeout, cancellation, or failed
output guard keeps Hex's corrected transcript. It never falls back to a cloud
request. The warm text process is shared, and turning the option off or
quitting Hex stops it and its native engine.

This version refines after recording ends. It does not implement Phonon's
background streaming refinement or promise Phonon's announced key-release latency.
The refiner is intended for English cleanup.

Phonon is GPL-3.0; its model uses the LFM Open License v1.0. Hex distributes
its own JSONL client, without Phonon source or weights. The installed Phonon
app supplies its sidecar, native engine, licenses, and notices. Model revision:
`2b0ab74bee7ab7cf364695bbe44f64ce4c47439e`.

Checks use a controlled JSONL fixture for readiness, final output, vocabulary,
style selection, malformed and large replies, deadlines, cancellation, and
blocked input pipes. These checks do not prove real-model quality, measured
latency, or text insertion into another app.

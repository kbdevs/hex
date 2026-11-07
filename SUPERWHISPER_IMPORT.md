# Superwhisper import and cloud rewriting

This fork adds an explicit local importer. It does not open Superwhisper,
read its account credentials, run its scripts, or send recordings to a model.
The importer is a CLI command; there is no import button in the window.

Build this branch using [LOCAL_TYPING.md](LOCAL_TYPING.md). Then run:

```sh
hex import superwhisper --dry-run --json
hex quit
hex import superwhisper
hex start
```

The source defaults to `~/Documents/superwhisper`. Use `--path` for another
Superwhisper data folder and `--archive` for another original-file archive.
The source must contain `recordings/<epoch-seconds>/meta.json`. This is the
folder format used by Superwhisper 2.x and 3.0.1. A SQLite-only installation
without these recording folders is not supported.

Dictation raw text and saved rewritten text enter Hex's normal History.
Recording dates, application names, mode names, and processing times are kept.
Paragraphs and Unicode are preserved. New local IDs do not change existing IDs,
and imported older entries appear below recent dictations. Repeating an import
skips retained entries with the same Superwhisper source ID. New IDs include
the canonical source folder's hash, so separate data folders can contain
recordings from the same second. Keep the source folder at the same path for
repeat imports. Existing archive reports preserve the older IDs for their
recorded source.

Meetings enter Hex's Meetings list with their existing titles, timed transcript
turns, and saved summaries. Named speakers appear in the transcript text. Hex's
generic You/Them headings do not identify an imported speaker. The importer does
not guess which speaker is the owner or split mixed audio into separate tracks.
Meeting imports publish complete folders atomically. They do not start model
inference, chapters, or summaries. Existing meeting retention settings apply;
with no explicit retention choice, imported meetings are kept.

New archives keep each source under
`~/Documents/Hex Imports/Superwhisper/sources/<source-hash>/recordings`.
An existing archive with a matching import report keeps its previous layout.
The command reports the archive path it uses.
This archive includes WAV and OGG audio, full metadata, user notes, empty
recordings, and any recordings that could not be decoded for the native lists.
Hex history retains its normal text and byte limits; the archive keeps complete
originals. History Off and time-based retention still apply. `--no-audio` omits
audio from the archive. The importer does not transcode or re-transcribe audio.

Local modes and meeting-template JSON files are also archived. Template prompts
are available through:

```sh
hex meetings templates list
hex meetings templates show starter.action-items
```

Importing a template saves its prompt; it does not add automatic template
execution or a template picker. Existing Hex settings stay as they are.

Before a history merge, the importer checks the existing file and saves a copy
under the archive's `hex-before-import` folder. Invalid Hex history or settings
stop the import. Sources and destinations must be separate folders. Copied files
are private on Unix. Symbolic links are not followed, and a changed archive file
causes an error instead of overwriting it. Successful files from a partial run
can be reused on the next run. `import-report.json` records counts and warnings.

## Cloud rewriting

Hex already supports cloud text rewriting through a running OpenCode service
and its configured providers. Speech recognition stays local. Rewriting is off
by default and sends the transcript and the context specified by Hex to that
provider. Superwhisper cloud subscriptions and account tokens do not transfer.

To enable it, read `dictation_processing.default_mode`, preserve the whole mode,
and change `post_processing.enabled` to `true` through `hex settings set`.
`post_processing.model` accepts a catalog key such as
`anthropic/claude-haiku-4-5-20251001`; `null` uses Hex's ranked available models.
The deadline defaults to 30 seconds. If rewriting fails, Hex keeps the local
transcript. Normal dictation still types the final text after recording ends.

## Validation

Run the importer, history, transcript CLI, and notes checks with:

```sh
RUSTUP_TOOLCHAIN=1.97.0 CARGO_PROFILE_DEV_STRIP=none CARGO_PROFILE_TEST_STRIP=none \
  scripts/check -E 'test(superwhisper_import) or test(history::) or test(control::transcript_cli) or test(meeting::notes)'
```

The tests use synthetic local files. They cover raw and rewritten text,
paragraphs, OGG and WAV copies, speaker names, summary attribution, orphan audio,
repeat imports, retention Off, malformed existing data, path overlap, links,
Unicode, metadata bounds, and owner-only permissions. A passing run does not
prove target-app keyboard input or cloud-provider availability.

#!/bin/bash
# Explicit first-time setup for the separately installed Phonon text refiner.
# No microphone, speech model, app UI, history, or user transcript is opened.
set -euo pipefail

phonon_contents=""
for candidate in /Applications/Phonon.app/Contents "$HOME/Applications/Phonon.app/Contents"; do
    if [[ -x "$candidate/Helpers/uv" && -f "$candidate/Resources/sidecar/polish_server.py" \
        && -f "$candidate/Resources/sidecar/linereader.py" \
        && -f "$candidate/Resources/sidecar/models/lfm3.py" \
        && -f "$candidate/Resources/prompts/polish_v2.txt" ]]; then
        phonon_contents="$candidate"
        break
    fi
done
if [[ -z "$phonon_contents" ]]; then
    echo 'Install Phonon first: brew install --cask infatoshi/tap/phonon' >&2
    exit 1
fi

phonon_resources="$phonon_contents/Resources"
phonon_command=(
    "$phonon_contents/Helpers/uv" run --no-project --python 3.12 --with mlx-lm==0.31.3 python
    "$phonon_resources/sidecar/polish_server.py"
    --model Infatoshi/phonon-refiner-1.7b
    --revision 2b0ab74bee7ab7cf364695bbe44f64ce4c47439e
    --system-prompt-file "$phonon_resources/prompts/polish_v2.txt"
)
if [[ -f "$phonon_contents/Helpers/pe" && -f "$phonon_resources/engine/mk.metal" && -f "$phonon_resources/engine/df.metal" ]]; then
    phonon_command+=(--engine-binary "$phonon_contents/Helpers/pe" --engine-shaders "$phonon_resources/engine")
fi

# Network access is explicit here. Normal dictation uses offline-only flags.
cd "$phonon_resources"
printf '%s\n' '{"command":"warmup"}' '{"command":"shutdown"}' |
    env UV_OFFLINE=false HF_HUB_OFFLINE=0 TRANSFORMERS_OFFLINE=0 PHONON_STREAM_REFINE=0 "${phonon_command[@]}" |
    env UV_OFFLINE=false "$phonon_contents/Helpers/uv" run --no-project --python 3.12 python -c '
import json, sys
ready = False
for line in sys.stdin:
    try:
        value = json.loads(line)
    except ValueError:
        print("Invalid setup response", file=sys.stderr)
        sys.exit(1)
    if not isinstance(value, dict):
        print("Invalid setup response", file=sys.stderr)
        sys.exit(1)
    if value.get("ok") is not True:
        print("Refiner setup failed: " + str(value.get("error") or "Check the installed Phonon version and network access."), file=sys.stderr)
        sys.exit(1)
    status = value.get("status", {})
    if not isinstance(status, dict):
        print("Invalid setup status", file=sys.stderr)
        sys.exit(1)
    if status.get("message"):
        print(status["message"], flush=True)
    ready = ready or status.get("state") == "ready"
if not ready:
    print("Refiner setup did not reach ready", file=sys.stderr)
    sys.exit(1)
print("Refiner ready. Turn on Local refiner (Phonon) in Hex Settings > Labs.")
'

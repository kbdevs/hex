#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$root/scripts/macos-signing.sh"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hex-local-signing.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
mkdir -p "$temporary/bin" "$temporary/repo/scripts"
cp "$root/scripts/build-app.sh" "$root/scripts/macos-signing.sh" "$temporary/repo/scripts/"
mkdir -p "$temporary/repo/app"
cp "$root/app/Info.plist" "$temporary/repo/app/Info.plist"
touch "$temporary/repo/LICENSE" "$temporary/repo/THIRD_PARTY_NOTICES.md"
cat > "$temporary/repo/scripts/build-hex-microphone-driver.sh" <<'SH'
#!/bin/sh
printf '%s:%s\n' "$HEX_TEAM_ID" "$HEX_CODESIGN_IDENTITY" > "$HEX_SIGNING_FIXTURE_DRIVER_PROFILE"
mkdir -p "$1/HexMicrophone.driver"
SH
chmod +x "$temporary/repo/scripts/build-hex-microphone-driver.sh"

cat > "$temporary/bin/security" <<'SH'
#!/bin/sh
if [ "${HEX_SIGNING_FIXTURE_NO_CERT:-}" != 1 ]; then
  echo '  1) AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA "Apple Development: Fixture (PERSON0000)"'
fi
echo '  2) BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB "Developer ID Application: Fixture (TESTTEAM00)"'
echo '  3) CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC "Apple Distribution: Fixture (TESTTEAM00)"'
echo '  4) DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD "Apple Development: Revoked (PERSON0000)" (CSSMERR_TP_CERT_REVOKED)'
SH
cat > "$temporary/bin/git" <<'SH'
#!/bin/sh
case "$*" in
  *' hex.localSigningIdentity') echo AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ;;
  *' hex.localSigningTeam')
    [ "${HEX_SIGNING_FIXTURE_NO_TEAM:-}" != 1 ] || exit 1
    echo TESTTEAM00 ;;
  *) exit 1 ;;
esac
SH
cat > "$temporary/bin/cargo" <<'SH'
#!/bin/sh
touch "$HEX_SIGNING_FIXTURE_CARGO_CALLED"
if [ "${HEX_SIGNING_FIXTURE_COMPLETE:-}" = 1 ]; then
  mkdir -p "$HEX_SIGNING_FIXTURE_ROOT/target/release"
  touch "$HEX_SIGNING_FIXTURE_ROOT/target/release/hex" "$HEX_SIGNING_FIXTURE_ROOT/target/release/scrawl-helper"
  exit 0
fi
exit 70
SH
cat > "$temporary/bin/xcrun" <<'SH'
#!/bin/sh
while [ "$#" -gt 0 ]; do
  if [ "$1" = --compile ]; then
    shift
    mkdir -p "$1"
    touch "$1/AppIcon.icns" "$1/Assets.car"
    exit 0
  fi
  shift
done
exit 1
SH
cat > "$temporary/bin/codesign" <<'SH'
#!/bin/sh
if [ "$1" = -d ]; then
  echo "TeamIdentifier=${HEX_SIGNING_FIXTURE_SIGNED_TEAM:-TESTTEAM00}" >&2
else
  printf '%s\n' "$*" >> "$HEX_SIGNING_FIXTURE_SIGN_CALLS"
fi
SH
chmod +x "$temporary/bin/security" "$temporary/bin/git" "$temporary/bin/cargo" "$temporary/bin/xcrun" "$temporary/bin/codesign"
PATH="$temporary/bin:$PATH"
export PATH

[ "$(hex_local_codesign_identity AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA)" = AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ]
[ "$(hex_local_codesign_identity bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb)" = bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb ]
for fingerprint in '' not-a-fingerprint CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC DDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDDD EEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEEE; do
  if hex_local_codesign_identity "$fingerprint" >/dev/null 2>&1; then
    echo "Unexpectedly accepted an invalid local signing identity." >&2
    exit 1
  fi
done

export HEX_LOCAL_BUILD=1 HEX_VERSION=3.3.0
export HEX_SIGNING_FIXTURE_CARGO_CALLED="$temporary/cargo-called"
export HEX_SIGNING_FIXTURE_ROOT="$temporary/repo"
export HEX_SIGNING_FIXTURE_DRIVER_PROFILE="$temporary/driver-profile"
export HEX_SIGNING_FIXTURE_SIGN_CALLS="$temporary/sign-calls"
unset HEX_CODESIGN_IDENTITY HEX_TEAM_ID HEX_TEST_BUILD HEX_RELEASE_UNSIGNED
expect_rejection() {
  if output=$("$@" 2>&1); then
    echo "Unexpectedly accepted a conflicting signing profile." >&2
    exit 1
  fi
  printf '%s\n' "$output" | grep -Fq "$expected"
  [ ! -e "$HEX_SIGNING_FIXTURE_CARGO_CALLED" ]
}
expected='Omit HEX_RELEASE_UNSIGNED'
expect_rejection env HEX_RELEASE_UNSIGNED=1 bash "$temporary/repo/scripts/build-app.sh"
expected='Signing overrides differ'
expect_rejection env HEX_TEAM_ID=WRONGTEAM0 bash "$temporary/repo/scripts/build-app.sh"
expect_rejection env HEX_CODESIGN_IDENTITY=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB bash "$temporary/repo/scripts/build-app.sh"
expected='Set both hex.localSigningIdentity'
expect_rejection env HEX_SIGNING_FIXTURE_NO_TEAM=1 bash "$temporary/repo/scripts/build-app.sh"
expected='Configured local signing certificate is unavailable'
expect_rejection env HEX_SIGNING_FIXTURE_NO_CERT=1 bash "$temporary/repo/scripts/build-app.sh"

set +e
bash "$temporary/repo/scripts/build-app.sh" >/dev/null 2>&1
result=$?
set -e
[ "$result" -eq 70 ] && [ -f "$HEX_SIGNING_FIXTURE_CARGO_CALLED" ]
rm "$HEX_SIGNING_FIXTURE_CARGO_CALLED"
expected='Signing identity is not a Developer ID Application identity'
expect_rejection env -u HEX_LOCAL_BUILD HEX_TEAM_ID=TESTTEAM00 HEX_CODESIGN_IDENTITY='Apple Development: Fixture (PERSON0000)' bash "$temporary/repo/scripts/build-app.sh"

HEX_SIGNING_FIXTURE_COMPLETE=1 bash "$temporary/repo/scripts/build-app.sh" >/dev/null 2>&1
[ -d "$temporary/repo/target/app/Hex.app" ]
[ "$(cat "$HEX_SIGNING_FIXTURE_DRIVER_PROFILE")" = TESTTEAM00:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ]
[ "$(grep -c -- '--sign AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA' "$HEX_SIGNING_FIXTURE_SIGN_CALLS")" -eq 3 ]
if output=$(HEX_SIGNING_FIXTURE_COMPLETE=1 HEX_SIGNING_FIXTURE_SIGNED_TEAM=WRONGTEAM0 bash "$temporary/repo/scripts/build-app.sh" 2>&1); then
  echo 'Unexpectedly accepted a bundle signed by another team.' >&2
  exit 1
fi
printf '%s\n' "$output" | grep -Fq 'does not match hex.localSigningTeam'
[ ! -e "$temporary/repo/target/app/Hex.app" ]
echo 'Local signing selection and release guards passed; no app was built.'

#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
. "$root/scripts/macos-signing.sh"
temporary=$(mktemp -d "${TMPDIR:-/tmp}/hex-local-signing.XXXXXX")
trap 'rm -rf "$temporary"' EXIT HUP INT TERM
mkdir -p "$temporary/bin" "$temporary/repo/scripts"
cp "$root/scripts/build-app.sh" "$root/scripts/macos-signing.sh" "$temporary/repo/scripts/"

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
exit 70
SH
chmod +x "$temporary/bin/security" "$temporary/bin/git" "$temporary/bin/cargo"
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
echo 'Local signing selection and release guards passed; no app was built.'

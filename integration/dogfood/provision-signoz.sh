#!/usr/bin/env bash
# Give the local SigNoz the shared dev credential and the checked-in dashboards
# (#956), so a fresh stack is immediately useful instead of asking every
# developer to sign up and then rebuild the same panels by hand.
#
# Safe to re-run. On a stack that is already provisioned this logs in, skips the
# dashboards it already imported, and changes nothing else.
#
# It deliberately does **not** rewrite an existing SigNoz account's password.
# Editing the credential store of a running service behind its own back is the
# kind of thing that works until it silently doesn't; when the existing account
# does not match, this says so and points at `just signoz-reset`, which discards
# SigNoz's metadata database on purpose and starts clean. Traces are unaffected
# either way — they live in ClickHouse, not in SigNoz's sqlite.
set -uo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE="${SIGNOZ_URL:-http://127.0.0.1:8080}"
# shellcheck source=/dev/null
set -a; . "$DIR/creds.env"; set +a

say() { printf '[signoz] %s\n' "$1"; }
die() { printf '[signoz] %s\n' "$1" >&2; exit 1; }

# read a top-level string/bool field out of a json response without jq
field() { python3 -c 'import json,sys
try: print(json.load(sys.stdin).get(sys.argv[1], ""))
except Exception: print("")' "$1" 2>/dev/null; }

# SigNoz serves the SPA (200 text/html) for any path that is not an API route,
# so "did this endpoint exist" cannot be answered by the status code alone —
# only a json content-type means an API answered.
post_json() {
  curl -s --max-time 15 -H 'Content-Type: application/json' \
       -H "${AUTH_HEADER:-X-Nothing: -}" -w '\n%{content_type}' \
       -X POST "$BASE$1" -d "$2" 2>/dev/null
}
is_json() { printf '%s' "$1" | tail -n1 | grep -qi 'application/json'; }
body() { printf '%s' "$1" | sed '$d'; }

say "waiting for $BASE"
for _ in $(seq 1 90); do
  curl -fsS --max-time 2 "$BASE/api/v1/version" >/dev/null 2>&1 && break
  sleep 2
done
version_json="$(curl -fsS --max-time 5 "$BASE/api/v1/version" 2>/dev/null)" \
  || die "not reachable at $BASE — is the stack up? (just dogfood)"
setup="$(printf '%s' "$version_json" | field setupCompleted)"

if [ "$setup" != "True" ] && [ "$setup" != "true" ]; then
  say "fresh instance — registering $DEV_EMAIL"
  out="$(post_json /api/v1/register \
    "$(python3 -c 'import json,os;print(json.dumps({
      "name": os.environ["DEV_NAME"],
      "email": os.environ["DEV_EMAIL"],
      "password": os.environ["DEV_PASSWORD"]}))')")"
  is_json "$out" || die "register did not answer as an api endpoint; SigNoz may have changed it"
  err="$(body "$out" | python3 -c 'import json,sys
try: print(json.load(sys.stdin).get("error",{}).get("message",""))
except Exception: print("")' 2>/dev/null)"
  [ -n "$err" ] && die "register refused: $err"
  say "registered"
  # most SigNoz builds hand back a session on register, which saves guessing at
  # the login route entirely — the path that matters for a fresh `just dogfood`
  REGISTER_TOKEN="$(body "$out" | field accessJwt)"
  [ -z "$REGISTER_TOKEN" ] && REGISTER_TOKEN="$(body "$out" | field accessToken)"
fi

# ── log in ───────────────────────────────────────────────────────────────────
# the login route has moved between SigNoz releases, so try the known spellings
# and use whichever actually answers as an api rather than hardcoding a guess
TOKEN="${REGISTER_TOKEN:-}"
ANSWERED=""
[ -n "$TOKEN" ] && say "using the session returned by register"
for path in /api/v1/login /api/v2/auth/login /api/v1/auth/login; do
  [ -n "$TOKEN" ] && break
  out="$(post_json "$path" \
    "$(python3 -c 'import json,os;print(json.dumps({
      "email": os.environ["DEV_EMAIL"],
      "password": os.environ["DEV_PASSWORD"]}))')")"
  is_json "$out" || continue
  ANSWERED="$path"
  tok="$(body "$out" | field accessJwt)"
  [ -z "$tok" ] && tok="$(body "$out" | field accessToken)"
  if [ -n "$tok" ]; then TOKEN="$tok"; say "logged in via $path"; break; fi
  msg="$(body "$out" | python3 -c 'import json,sys
try: print(json.load(sys.stdin).get("error",{}).get("message",""))
except Exception: print("")' 2>/dev/null)"
  [ -n "$msg" ] && LOGIN_ERR="$msg"
done

if [ -z "$TOKEN" ] && [ -z "$ANSWERED" ]; then
  # every candidate fell through to the SPA, so none of them is a login route on
  # this build. say that, rather than blaming the password for it.
  cat >&2 <<EOF
[signoz] no login endpoint answered on $BASE

  Tried /api/v1/login, /api/v2/auth/login and /api/v1/auth/login; each returned
  the SPA rather than json, which means SigNoz $(printf '%s' "$version_json" | field version) spells it
  differently. The credential was not changed and nothing was imported.

  Add the correct path to the candidate list in this script. Until then, import
  the dashboards by hand: SigNoz → Dashboards → Import JSON, using the files in
  integration/dogfood/signoz/dashboards/.
EOF
  exit 1
fi

if [ -z "$TOKEN" ]; then
  cat >&2 <<EOF
[signoz] could not log in as $DEV_EMAIL${LOGIN_ERR:+ ($LOGIN_ERR)}

  This instance already has an account that is not the shared dev credential.
  Nothing was changed. Either sign in with the password you chose, or discard
  SigNoz's metadata and let this script set it up cleanly:

      just signoz-reset

  Your traces are in ClickHouse and survive that — only SigNoz's own users,
  dashboards and alerts live in the database it removes.

  The dashboards can also be imported by hand: SigNoz → Dashboards → Import
  JSON, using the files in integration/dogfood/signoz/dashboards/.
EOF
  exit 1
fi
AUTH_HEADER="Authorization: Bearer $TOKEN"

# ── dashboards ───────────────────────────────────────────────────────────────
existing="$(curl -s --max-time 10 -H "$AUTH_HEADER" "$BASE/api/v1/dashboards" 2>/dev/null \
  | python3 -c 'import json,sys
try:
    d = json.load(sys.stdin)
    rows = d.get("data", d) if isinstance(d, dict) else d
    for r in rows:
        t = (r.get("data") or {}).get("title") or r.get("title") or ""
        if t: print(t)
except Exception: pass' 2>/dev/null)"

imported=0 skipped=0 failed=0
for f in "$DIR"/signoz/dashboards/*.json; do
  [ -e "$f" ] || continue
  title="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("title",""))' "$f")"
  if printf '%s\n' "$existing" | grep -Fxq "$title"; then
    skipped=$((skipped + 1)); continue
  fi
  out="$(curl -s --max-time 20 -H 'Content-Type: application/json' -H "$AUTH_HEADER" \
        -w '\n%{content_type}' -X POST "$BASE/api/v1/dashboards" --data-binary "@$f" 2>/dev/null)"
  if is_json "$out" && ! body "$out" | grep -q '"error"'; then
    imported=$((imported + 1))
  else
    failed=$((failed + 1))
    say "could not import '$title' — import it by hand from ${f#"$DIR"/}"
  fi
done

say "dashboards: $imported imported, $skipped already present, $failed failed"
[ "$failed" -gt 0 ] && exit 1
exit 0

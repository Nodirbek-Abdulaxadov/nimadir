#!/usr/bin/env bash
# The test suite. There is no `cargo test` here on purpose: what needs proving
# is the host<->guest boundary and the shell's routing, and both are exercised
# end to end through the real binary, headlessly, with no display.
#
#   ./run-tests.sh          run everything
#   ./run-tests.sh -v       print each command's full output
#
# Network cases need `python3` for a throwaway HTTP server; they are skipped
# cleanly without it, so the suite still runs anywhere the shell itself does.
set -uo pipefail
cd "$(dirname "$0")"

VERBOSE=${1:-}
PASS=0; FAIL=0; SKIP=0
PORT=${NIMADIR_TEST_PORT:-8899}
SRV=""
FIXTURES=""

red()   { printf '\033[31m%s\033[0m' "$1"; }
green() { printf '\033[32m%s\033[0m' "$1"; }
dim()   { printf '\033[2m%s\033[0m'  "$1"; }

# check <name> <expected substring> <command...>
check() {
  local name="$1" want="$2"; shift 2
  local got status
  got=$("$@" 2>&1); status=$?
  if [[ "$got" == *"$want"* ]]; then
    green "  PASS"; echo "  $name"
    PASS=$((PASS + 1))
  else
    red "  FAIL"; echo "  $name"
    echo "         wanted: $want"
    echo "         exit:   $status"
    echo "$got" | sed 's/^/         > /' | head -12
    FAIL=$((FAIL + 1))
  fi
  [[ -n "$VERBOSE" ]] && echo "$got" | sed 's/^/         | /'
  return 0
}

# check_fails <name> <expected substring> <command...> — must exit non-zero.
check_fails() {
  local name="$1" want="$2"; shift 2
  local got status
  got=$("$@" 2>&1); status=$?
  if [[ $status -ne 0 && "$got" == *"$want"* ]]; then
    green "  PASS"; echo "  $name"
    PASS=$((PASS + 1))
  else
    red "  FAIL"; echo "  $name  (exit $status, wanted non-zero + \"$want\")"
    echo "$got" | sed 's/^/         > /' | head -8
    FAIL=$((FAIL + 1))
  fi
  return 0
}

skip() { printf '  '; dim "SKIP"; echo "  $1"; SKIP=$((SKIP + 1)); }

cleanup() {
  [[ -n "$SRV" ]] && kill "$SRV" 2>/dev/null
  [[ -n "$FIXTURES" ]] && rm -rf "$FIXTURES"
  return 0
}
trap cleanup EXIT

HOST=./target/debug/host

echo "==> building"
cargo build -q -p host || { red "host build failed"; echo; exit 1; }
if [[ ! -f mini-apps/store/store.component.wasm ]]; then
  echo "    mini-apps missing; running build-mini-apps.sh"
  ./build-mini-apps.sh >/dev/null || { red "mini-app build failed"; echo; exit 1; }
fi

echo
echo "==> the host<->guest cycle"
# The count lives in the guest: clicks in, redrawn labels out.
check "counter counts and resets" '"Count: 3"' \
  $HOST counter --script "1:0,2:0,3:0,5:1" --frames 8
check "counter resets to 0 after the reset click" '"Count: 0"' \
  $HOST counter --script "1:0,2:0,3:0,5:1" --frames 8
check "a second component runs on the same host" "Hello from a *different* mini-app!" \
  $HOST hello --frames 1
check "logs cross the boundary" "[wasm log]" \
  $HOST counter --frames 1

echo
echo "==> the registry"
check "a bare name resolves to its source" "mini-apps/counter/counter.component.wasm" \
  $HOST --list
check "--list names where the entries came from" "apps.json" \
  $HOST --list
check "an unreachable registry falls back and says so" "built-in" \
  $HOST --registry http://127.0.0.1:9/nope.json --list

echo
echo "==> the store (start page as a mini-app)"
check "lists the registry through list-apps" '"4 apps"' \
  $HOST store --frames 1
check "draws a search field" "field0" \
  $HOST store --frames 1
check "filters as you type" '1 of 4 apps matching' \
  $HOST store --input "1:0=hell" --frames 2
check "renumbers buttons when the list shrinks" '"Open hello"' \
  $HOST store --input "1:0=hell" --frames 2
check "open-app navigates after the frame returns" "[nav] loaded" \
  $HOST store --input "1:0=hell" --script "2:0" --frames 4
check "the app it asked for actually runs" "Hello from a *different* mini-app!" \
  $HOST store --input "1:0=hell" --script "2:0" --frames 4
check "a search matching nothing says so" "Nothing matches" \
  $HOST store --input "1:0=zzzz" --frames 2

echo
echo "==> address classification"
check "a local .wasm is a component" "loaded 17643 bytes" \
  $HOST mini-apps/counter/counter.component.wasm --frames 1
check_fails "a local file that is neither is refused, without a network call" \
  "expected a .wasm component or an .html page" $HOST README.md
# Named .wasm but not one: classification lets it through, the parser stops it.
BOGUS=$(mktemp -d)/bogus.wasm
printf 'this is not webassembly\n' > "$BOGUS"
check_fails "a file named .wasm that is not one fails at parse, not at classification" \
  "failed to parse WebAssembly module" $HOST "$BOGUS"
rm -rf "$(dirname "$BOGUS")"

echo
echo "==> address bar guessing"
# The guess is last: a registry name and a real file both beat it.
check "a registry name wins over any guess" "mini-apps/counter/counter.component.wasm" \
  $HOST counter --frames 1
check_fails "host-shaped text gets https://" "https://example.com" \
  $HOST example.com
check_fails "a filename is never guessed as a host" "read file app.wasm" \
  $HOST app.wasm

if ! command -v python3 >/dev/null 2>&1; then
  echo
  echo "==> network cases"
  skip "python3 not on PATH, no throwaway server"
else
  FIXTURES=$(mktemp -d)
  cp mini-apps/counter/counter.component.wasm "$FIXTURES/"
  # No extension and served as octet-stream: only the magic number can settle it.
  cp mini-apps/counter/counter.component.wasm "$FIXTURES/mystery"
  printf '<!doctype html><title>t</title><p>hello\n' > "$FIXTURES/page.html"
  printf 'not a component and not a page\n'          > "$FIXTURES/notes.txt"
  cat > "$FIXTURES/catalogue.json" <<JSON
[{"name":"hello","source":"mini-apps/hello/hello.component.wasm","description":"served"}]
JSON
  # `exec` so $! is python itself: killing a wrapper subshell would leave the
  # server running, and the next run would find the port taken and report a
  # screenful of failures that have nothing to do with the code.
  (cd "$FIXTURES" && exec python3 -m http.server $PORT --bind 127.0.0.1) >/dev/null 2>&1 &
  SRV=$!

  READY=""
  for _ in $(seq 1 40); do
    if curl -sf --noproxy '*' -o /dev/null "http://127.0.0.1:$PORT/page.html"; then
      READY=1; break
    fi
    kill -0 "$SRV" 2>/dev/null || break   # it exited: port in use, most likely
    sleep 0.25
  done

  echo
  echo "==> network cases (Content-Type routing)"
  if [[ -z "$READY" ]]; then
    # Never report these as failures: nothing was tested, and saying "failed"
    # would send someone looking for a bug in the router.
    skip "could not start a server on 127.0.0.1:$PORT (port in use?)"
  else
    B="http://127.0.0.1:$PORT"
    check "application/wasm is a component" "loaded 17643 bytes" \
      $HOST "$B/counter.component.wasm" --frames 1
    check "octet-stream with no extension: the magic number settles it" "loaded 17643 bytes" \
      $HOST "$B/mystery" --frames 1
    check "text/html routes to the web page module" "is a web page" \
      $HOST "$B/page.html"
    check_fails "text/plain is neither, and says which it got" "Content-Type: text/plain" \
      $HOST "$B/notes.txt"
    check "a registry can be served over HTTP" "served" \
      $HOST --registry "$B/catalogue.json" --list
    # The store lists whatever registry the host was given, not a compiled-in one.
    check "a served registry is what the store lists" '"1 apps"' \
      $HOST --registry "$B/catalogue.json" mini-apps/store/store.component.wasm --frames 1
    check "and its entries are the served ones" "served" \
      $HOST --registry "$B/catalogue.json" mini-apps/store/store.component.wasm --frames 1
  fi
fi

echo
echo "==> builds"
check "headless (default) builds" "" cargo build -q -p host
if pkg-config --exists gtk+-3.0 2>/dev/null || [[ "$OSTYPE" != linux* ]]; then
  check "gui builds" "" cargo build -q -p host --features gui
else
  skip "gui build (no system GUI libraries here)"
fi
if pkg-config --exists webkit2gtk-4.1 2>/dev/null || [[ "$OSTYPE" != linux* ]]; then
  check "webview builds" "" cargo build -q -p host --features webview
else
  skip "webview build (no WebKitGTK; install libwebkit2gtk-4.1-dev)"
fi

echo
echo "-----------------------------------------------"
printf '  '; green "$PASS passed"
[[ $SKIP -gt 0 ]] && { printf ', '; dim "$SKIP skipped"; }
if [[ $FAIL -gt 0 ]]; then printf ', '; red "$FAIL failed"; echo; echo; exit 1; fi
echo; echo

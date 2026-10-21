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

# check <name> <expected substring> <command...> — must exit zero AND match.
#
# The status matters as much as the text: with an empty `want` (the build
# checks) a substring test alone matches anything, so a failing build passed.
check() {
  local name="$1" want="$2"; shift 2
  local got status
  got=$("$@" 2>&1); status=$?
  if [[ $status -eq 0 && "$got" == *"$want"* ]]; then
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

# check_absent <name> <substring that must NOT appear> <command...> — must also
# exit zero, or "it isn't there" would be satisfied by the thing never running.
check_absent() {
  local name="$1" bad="$2"; shift 2
  local got status
  got=$("$@" 2>&1); status=$?
  if [[ $status -eq 0 && "$got" != *"$bad"* ]]; then
    green "  PASS"; echo "  $name"
    PASS=$((PASS + 1))
  else
    red "  FAIL"; echo "  $name"
    echo "         did not want: $bad"
    echo "         exit:         $status"
    echo "$got" | sed 's/^/         > /' | head -12
    FAIL=$((FAIL + 1))
  fi
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
  [[ -n "${NIMADIR_HOME:-}" ]] && rm -rf "$NIMADIR_HOME"
  return 0
}
trap cleanup EXIT

# The shell reads and writes its config, data and caches under here instead of
# the real ones of whoever runs the suite — and starts every run empty.
export NIMADIR_HOME
NIMADIR_HOME=$(mktemp -d)

# A path the host itself can open. Under Git Bash on Windows `$PWD` and
# `mktemp` give MSYS paths (`/c/...`, `/tmp/...`) that only bash understands;
# the host is a native program. Elsewhere this is the path unchanged.
native() {
  if command -v cygpath >/dev/null 2>&1; then cygpath -m "$1"; else printf '%s' "$1"; fi
}
NIMADIR_HOME=$(native "$NIMADIR_HOME")
# Off: a shell with no registry would otherwise fetch the project's hosted one,
# and the suite must not depend on the internet. The cases that need it point
# it at the local server.
export NIMADIR_HOSTED_REGISTRY=""

HOST=./target/debug/nimadir-shell

echo "==> building"
# Mini-apps first: the host carries the store inside it when the store exists
# at the time it is built.
if [[ ! -f mini-apps/store/store.component.wasm || ! -f mini-apps/runaway/runaway.component.wasm || ! -f mini-apps/future/future.component.wasm || ! -f mini-apps/fetcher/fetcher.component.wasm || ! -f mini-apps/fuzz/fuzz.component.wasm || ! -f mini-apps/todo/todo.component.wasm || ! -f mini-apps/probe/probe.component.wasm ]]; then
  echo "    mini-apps missing; running build-mini-apps.sh"
  ./build-mini-apps.sh >/dev/null || { red "mini-app build failed"; echo; exit 1; }
fi
cargo build -q -p host -p nimadir-cli || { red "host build failed"; echo; exit 1; }

# The size of the component as just built, not a number typed in here: a
# different rustc emits a different-sized component, and a hardcoded byte count
# turns a routine toolchain bump into four red lines about nothing. What these
# assertions are for is "the bytes that arrived are the component that was
# built" — so ask the file.
COUNTER_BYTES=$(wc -c < mini-apps/counter/counter.component.wasm | tr -d ' ')

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
# A terminal takes escape sequences as commands; a guest's log must not reach
# one raw. The fuzz fixture logs "\0\n\t" and a right-to-left override.
check "a guest's control characters reach the terminal escaped" '\u{202e}' \
  bash -c "set -o pipefail; $HOST mini-apps/fuzz/fuzz.component.wasm --frames 30 | tr -d '\\000'"
# Polyglot, on the host that was never told which language it is running.
# Skipped rather than failed when the optional toolchain is absent — the same
# treatment `build-mini-apps.sh` gives it.
if [[ -f mini-apps/counter-py/counter-py.component.wasm ]]; then
  check "a Python guest counts on the same host" '"2"' \
    $HOST counter-py --script "1:0,2:0" --frames 4
  # CPython's own standard library, running inside the sandbox.
  check "Python's stdlib runs in the sandbox" "host clock via Python datetime" \
    $HOST counter-py --frames 1
else
  skip "counter-py (Python): component not built"
fi

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
# Built on the element tree (ADR-4): a search field (node 5), a card per app
# — an <a href> the shell follows — counter #17, hello #24; a card offering a
# typed address (#7); the settings link (#15). Searching hides cards rather
# than rebuilding them, so a card keeps its node.
check "lists the registry through list-apps" '"6 apps"' \
  $HOST store --frames 1
# 7 entries in apps.json, 6 offered: a start page that lists itself is a card
# that takes you where you already are.
check_absent "the start page is not one of its own cards" '"store"' \
  $HOST store --frames 1
check "draws a search field" '#5 input#q [type="search"' \
  $HOST store --frames 1
check "the start page leads with a hero heading" '#3 h1.wordmark' \
  $HOST store --frames 1
check "filters as you type" '1 of 6 apps matching' \
  $HOST store --type "1:5=hell" --frames 2
check "hides what does not match, each card keeping its node" '#17 a.tile [href="mini-apps/counter/counter.component.wasm" hidden=""]' \
  $HOST store --type "1:5=hell" --frames 2
check "a card navigates after the frame returns" "[nav] loaded" \
  $HOST store --type "1:5=hell" --click "2:24" --frames 4
check "the app it asked for actually runs" "Hello from a *different* mini-app!" \
  $HOST store --type "1:5=hell" --click "2:24" --frames 4
check "Enter opens the one app that matches" "Hello from a *different* mini-app!" \
  $HOST store --type "1:5=hell" --enter "2:5" --frames 4
check "the start page links to the settings page" '#15 a.settings [href="nimadir:settings"]' \
  $HOST store --frames 1
check "which opens as host UI" "[nav] now at Settings" \
  $HOST store --click "1:15" --frames 3
check "a search matching nothing says so" "Nothing matches" \
  $HOST store --type "1:5=zzzz" --frames 2
# The start page has no address bar over it, so its own field has to take an
# address too — offered as a card, never acted on while you are still typing.
check "a typed address is offered as a card" '#7 a#open.tile.open [href="example.com"]' \
  $HOST store --type "1:5=example.com" --frames 2
# A local path, not a web address: this is about the store handing an address
# to the shell, and it must not depend on reaching the internet.
check "and tapping that card navigates" "[nav] loaded" \
  $HOST store --type "1:5=mini-apps/hello/hello.component.wasm" --click "2:7" --frames 3
check "a guest navigation that fails says so" "[nav failed] could not open nowhere/missing.wasm" \
  $HOST store --type "1:5=nowhere/missing.wasm" --click "2:7" --frames 3

echo
echo "==> outside a checkout"
# An installed shell has no mini-apps/ and no apps.json beside it. It must
# still open on a start page, and read its registry from where it lives.
ELSEWHERE=$(mktemp -d)
ABS_HOST="$PWD/target/debug/nimadir-shell"
check "started anywhere, the shell opens the store built into it" '#3 h1.wordmark' \
  bash -c "cd '$ELSEWHERE' && '$ABS_HOST' store --frames 1"
mkdir -p "$NIMADIR_HOME/config"
printf '[{"name":"hello","source":"%s","description":"from the config directory"}]\n' \
  "$(native "$PWD/mini-apps/hello/hello.component.wasm")" > "$NIMADIR_HOME/config/apps.json"
check "an installed shell reads the registry in its config directory" "from the config directory" \
  bash -c "cd '$ELSEWHERE' && '$ABS_HOST' --list"
check "and the built-in store lists what that registry holds" '"1 apps"' \
  bash -c "cd '$ELSEWHERE' && '$ABS_HOST' store --frames 1"
rm -f "$NIMADIR_HOME/config/apps.json"
# A fresh phone before the hosted registry arrives: only the store itself.
check "with nothing listed, the store says so rather than blame the search" "No apps are listed yet" \
  bash -c "cd '$ELSEWHERE' && '$ABS_HOST' store --frames 1"
check "NIMADIR_REGISTRY names the registry for every run" "mini-apps/counter/counter.component.wasm" \
  bash -c "cd '$ELSEWHERE' && NIMADIR_REGISTRY='$(native "$PWD/apps.json")' '$ABS_HOST' --list"
rm -rf "$ELSEWHERE"

echo
echo "==> signed apps"
# ADR-1: a component carries its manifest and its publisher's signature.
PACK=./target/debug/nimadir
SIGNED=$(mktemp -d)
PUBLISHER=$($PACK keygen "$SIGNED/key" | awk '{print $2}')
printf '{"name":"hello","version":"1.0.0"}\n' > "$SIGNED/manifest.json"
$PACK pack mini-apps/hello/hello.component.wasm "$SIGNED/manifest.json" "$SIGNED/signed.wasm" --key "$SIGNED/key" >/dev/null
$PACK pack mini-apps/hello/hello.component.wasm "$SIGNED/manifest.json" "$SIGNED/unsigned.wasm" >/dev/null
cp "$SIGNED/signed.wasm" "$SIGNED/tampered.wasm"
printf 'XXXX' | dd of="$SIGNED/tampered.wasm" bs=1 seek=2000 conv=notrunc 2>/dev/null
check "a signed app says who signed it" "hello 1.0.0, signed by $PUBLISHER" \
  $HOST "$(native "$SIGNED/signed.wasm")" --frames 1
check "nimadir names the identity it claims" "identity  $PUBLISHER/hello" \
  $PACK inspect "$SIGNED/signed.wasm"
check_fails "a signed app changed after signing is refused" "changed after it was signed" \
  $HOST "$(native "$SIGNED/tampered.wasm")" --frames 1
check "an app with a manifest and no signature runs, unverified" "hello 1.0.0, unsigned" \
  $HOST "$(native "$SIGNED/unsigned.wasm")" --frames 1
printf '[{"name":"hello","source":"%s","publisher":"someoneelse"}]\n' \
  "$(native "$SIGNED/signed.wasm")" > "$SIGNED/registry.json"
check_fails "an app signed by another publisher than its listing says is refused" "is listed as published by someoneelse" \
  $HOST --registry "$(native "$SIGNED/registry.json")" hello --frames 1
rm -rf "$SIGNED"

echo
echo "==> storage"
# ADR-2: kept between visits for an app signed and asking for it; otherwise
# it ends with the app, like the rest of its state.
KEPT=$(mktemp -d)
KEPT_PUBLISHER=$($PACK keygen "$KEPT/key" | awk '{print $2}')
printf '{"name":"counter","version":"1.0.0","permissions":["storage"]}\n' > "$KEPT/asks.json"
printf '{"name":"counter","version":"1.0.0"}\n' > "$KEPT/silent.json"
$PACK pack mini-apps/counter/counter.component.wasm "$KEPT/asks.json" "$KEPT/kept.wasm" --key "$KEPT/key" >/dev/null
$PACK pack mini-apps/counter/counter.component.wasm "$KEPT/silent.json" "$KEPT/silent.wasm" --key "$KEPT/key" >/dev/null
$HOST "$(native "$KEPT/kept.wasm")" --script "1:0,2:0" --frames 4 >/dev/null
check "a signed app that asks for storage finds it on the next visit" '"Count: 2"' \
  $HOST "$(native "$KEPT/kept.wasm")" --frames 1
$HOST "$(native "$KEPT/silent.wasm")" --script "1:0,2:0" --frames 4 >/dev/null
check "one that does not ask starts afresh" '"Count: 0"' \
  $HOST "$(native "$KEPT/silent.wasm")" --frames 1
check "and an unsigned app keeps nothing" '"Count: 0"' \
  bash -c "$HOST counter --script 1:0 --frames 3 >/dev/null && $HOST counter --frames 1"
check "--kept lists what each app keeps" "$KEPT_PUBLISHER/counter" \
  $HOST --kept
check "--clear removes it, and the next visit starts afresh" '"Count: 0"' \
  bash -c "$HOST --clear $KEPT_PUBLISHER/counter >/dev/null && $HOST '$(native "$KEPT/kept.wasm")' --frames 1"
check_fails "--clear refuses anything that is not an identity" "is not an app identity" \
  $HOST --clear "../../etc"
check "storage past the quota is refused, with the reason, not a trap" "storage is full" \
  $HOST mini-apps/runaway/runaway.component.wasm --script "1:3" --frames 3
rm -rf "$KEPT"

echo
echo "==> trusted downloads"
# What crosses the network unencrypted could have been swapped on the way.
check_fails "plain http to another machine is refused, before any request" "plain http" \
  $HOST http://example.invalid/app.wasm
# A registry can pin each component's SHA-256; bytes that differ never run.
sha256_of() {
  sha256sum "$1" 2>/dev/null | cut -d' ' -f1 && return
  shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1 && return
  python3 -c "import hashlib,sys; print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$1"
}
COUNTER_SHA=$(sha256_of mini-apps/counter/counter.component.wasm)
HELLO_SHA=$(sha256_of mini-apps/hello/hello.component.wasm)
PINS=$(mktemp -d)
HELLO_NATIVE=$(native "$PWD/mini-apps/hello/hello.component.wasm")
printf '[{"name":"hello","source":"%s","sha256":"%s"}]\n' "$HELLO_NATIVE" "$COUNTER_SHA" > "$PINS/tampered.json"
printf '[{"name":"hello","source":"%s","sha256":"%s"}]\n' "$HELLO_NATIVE" "$HELLO_SHA" > "$PINS/pinned.json"
check_fails "a component that is not the one pinned is refused" "is not the component the registry pinned" \
  $HOST --registry "$(native "$PINS/tampered.json")" hello --frames 1
check "the one pinned runs" "Hello from a *different* mini-app!" \
  $HOST --registry "$(native "$PINS/pinned.json")" hello --frames 1
rm -rf "$PINS"

echo
echo "==> the shell's own record"
# Read off the WIT, so the next minor version does not turn this red.
WIT_PACKAGE=$(sed -n 's/^package \(.*\);/\1/p' wit/world.wit)
check "--version names the host and the WIT it implements" "(WIT $WIT_PACKAGE)" \
  $HOST --version
check "navigations are logged in the data directory" "bytes of wasm from mini-apps/runaway/runaway.component.wasm" \
  bash -c "$HOST mini-apps/runaway/runaway.component.wasm --frames 1 >/dev/null && cat \"\$NIMADIR_HOME/data/nimadir.log\""
check "a guest that is stopped is logged with the reason" "the app stopped: it did not finish a frame" \
  bash -c "$HOST mini-apps/runaway/runaway.component.wasm --script 1:0 --frames 3 >/dev/null 2>&1; cat \"\$NIMADIR_HOME/data/nimadir.log\""

echo
echo "==> navigation history"
# Back reopens the page it returns to — leaving an app destroyed it.
check "back from an app the store opened returns to the store" "[back] now at store" \
  $HOST store --type "1:5=hell" --click "2:24" --back 4 --frames 6
check "back from an app opened directly goes to the start page" "[back] now at store" \
  $HOST counter --back 1 --frames 2
# The store opens apps by their source; the title is still the app's name,
# not a path (on a phone, one deep in private storage).
check "an app the store opened is titled by its name" "[back] now at hello" \
  $HOST store --click "1:24" --script "3:1" --back 5 --frames 6
# On a phone this is what sends the app to the background.
check "back from the start page has nowhere to go" "[back] nothing to go back to" \
  $HOST store --back 1 --frames 2
check "reload starts the app afresh" '"Count: 0"' \
  bash -c "$HOST counter --script 1:0,2:0 --reload 4 --frames 5 | sed -n '/reload/,\$p'"

echo
echo "==> where an app may send you"
# A listed app opens on the app's say-so; anywhere else, the user decides.
check "an app can open an app the registry lists" "[nav] loaded" \
  $HOST hello --script "1:1" --frames 3
check "but asks before opening an address the registry does not list" "[asks] hello wants to open https://example.com" \
  $HOST hello --script "1:2" --frames 3
check "and without a yes, stays where it is" "[declined]" \
  $HOST hello --script "1:2" --frames 3

echo
echo "==> the widget vocabulary"
# gallery uses every widget. Taps: button0, check1. Values: field0, choice1.
check "groups nest, and say what they are" "begin section#0" \
  $HOST gallery --frames 1
check "a checkbox answers its tap" 'check1 : [x] "Remember me"' \
  $HOST gallery --script "1:1" --frames 2
check "a choice answers its pick" '"Size: Large"' \
  $HOST gallery --input "1:1=2" --frames 2
check_absent "a pick out of range is ignored" "Size: Large" \
  $HOST gallery --input "1:1=9" --frames 2
check "a field is acted on when submitted" '"Hello, Ada!"' \
  $HOST gallery --input "1:0=Ada" --submit "2:0" --frames 3
check_absent "and not on every key before it" "Hello, Ada!" \
  $HOST gallery --input "1:0=Ada" --frames 3
check "groups nested past the limit are drawn flat, not refused" '"deep"' \
  $HOST mini-apps/runaway/runaway.component.wasm --script "1:4" --frames 2
check "an image is handed over once and drawn by number" "image : 48x48" \
  $HOST gallery --frames 1
check "an image that claims to be huge is refused from its header" "the most is 4096 on a side" \
  $HOST mini-apps/runaway/runaway.component.wasm --script "1:5" --frames 2

echo
echo "==> the element tree (ADR-4)"
# Built once in init, edited by events: the page outlives the frame.
TODO=mini-apps/todo/todo.component.wasm
check "an app builds a page as a tree of elements" '#9 ul.list {click change}' \
  $HOST $TODO --frames 1
check "typing and Enter reach the app as events, and it adds a node" '"Buy milk"' \
  $HOST $TODO --type "1:6=Buy milk" --enter "1:6" --frames 2
check "a click on a checkbox flips it and reaches its list's listener" 'li.item.done' \
  $HOST $TODO --click "1:12" --frames 2
check "a click on a button's text is a click on the button" '"1 thing left"' \
  $HOST $TODO --click "1:16" --frames 2
# Released with what is under it: the item's six nodes, 23 down to 17.
check "a released node goes with everything under it" '[page] 17 nodes' \
  $HOST $TODO --click "1:16" --frames 2

echo
echo "==> CSS (ADR-4)"
# The probe's page takes whatever stylesheet is typed into node 2; --styles
# prints what each element's rules set.
PROBE=mini-apps/probe/probe.component.wasm
css() { $HOST $PROBE --frames 2 --styles --type "1:2=$1" "${@:2}"; }
check "the host's own sheet gives elements a look" '#3 h1 | font-size: 28px; font-weight: 700' \
  $HOST $TODO --styles --frames 1
check "and follows the system's dark theme" 'background: #303134; font-weight: 600' \
  $HOST $TODO --styles --dark --frames 1
check "an id outweighs a class" '#7 p | margin: 0px 0px 12px; color: #ff0000' \
  css '#a p { color: red } .box p { color: blue }'
check "!important outweighs an id" '#5 p.first | margin: 0px 0px 12px; color: #008000' \
  css '.first { color: green !important } #a p { color: red }'
check "inline style outweighs a sheet" '#23 span.note [style="letter-spacing: 1px"] | letter-spacing: 1px' \
  css '.note { letter-spacing: 3px }'
check "siblings, :nth-child and :not match as on the web" '#16 li | padding: 2px; font-weight: 700' \
  css 'li:nth-child(2n+1) { font-weight: 700 } li:not(:first-child):not(:last-child) { padding: 2px }'
check "attribute selectors match" '#9 p.last [lang="uz-UZ"] | margin: 0px 0px 12px; color: #00aa00' \
  css '[lang|=uz] { color: #0a0 }'
check "var() with a fallback, calc(), em and rem resolve" '#23 span.note [style="letter-spacing: 1px"] | margin: 3px; padding: 12px; color: #008000; font-size: 12px' \
  css ':root { --x: 12px; --c: hsl(120, 100%, 25%) } .note { padding: var(--x); margin: var(--missing, 3px); color: var(--c); font-size: 0.8rem }'
check "calc() keeps a percentage for layout" 'width: calc(100% + -32px); padding: 20px; font-size: 20px' \
  css '.box { width: calc(100% - 32px); font-size: 20px; padding: 1em }'
check "@media follows the page's width" 'grid-template-columns: repeat(3, 1fr) 200px' \
  css '@media (max-width: 400px) { #page { display: grid; grid-template-columns: repeat(3, 1fr) 200px } }' --viewport 360x640
check_absent "and leaves a wider page alone" '#3 div#page | display: grid' \
  css '@media (max-width: 400px) { #page { display: grid } }'
check "what a sheet cannot say is skipped, logged, and the rest applies" 'css: skipped `p::before`' \
  css 'p::before { color: red } .first { color: blue }'
check "query-all finds by selector, in document order" 'query "#a > p:not(.last), li:last-child": [5, 7, 18]' \
  $HOST $PROBE --frames 2 --type "1:25=#a > p:not(.last), li:last-child"

echo
echo "==> layout and paint (ADR-4)"
# Laid out with stand-in fonts (`layout::Approx`), so boxes are the same on
# every machine; --boxes prints each node's, --paint what would be drawn.
# The sheet is typed before frame 0, so no frame shows the page without it.
lay() { $HOST $PROBE --frames 1 --boxes --viewport 410x700 --type "0:2=$1"; }
check "a page is laid out: flex, padding, max-width" '#6 input [placeholder="What needs doing?"] value="" {submit} @16,74 300x39' \
  $HOST $TODO --boxes --viewport 400x700 --frames 1
check "grid tracks and gaps" '#11 ul @210,82 200x' \
  lay '#page { display: grid; grid-template-columns: 1fr 1fr; gap: 10px }'
check "calc() resolves against the box it sits in" '#4 div#a.box.wide @0,82 168x' \
  lay '#page { display: grid; grid-template-columns: 1fr 1fr; gap: 10px } .box { width: calc(100% - 32px) }'
check "absolute positioning from the containing block" '#23 span.note [style="letter-spacing: 1px"] @306,90 ' \
  lay '.note { position: absolute; right: 20px; top: 90px }'
check_absent "display: none takes a node out of the layout" '"one" @' \
  lay '.box { display: none }'
check "an overflowing box clips what is in it" '[paint] clip 0,82 800x20' \
  $HOST $PROBE --paint --frames 1 --type '0:2=.box { display: none } ul { overflow: auto; height: 20px }'
check "a checked checkbox draws its tick" '[paint] check 55,125 18x18 on' \
  $HOST $PROBE --paint --frames 1 --type '0:2=.box { display: none } ul { overflow: auto; height: 20px }'
check "bounds() answers from the last layout" 'bounds Some(Rect { x: 0.0, y: 82.0, width: 400.0' \
  $HOST $PROBE --click "1:20" --frames 2 --viewport 400x700

echo
echo "==> forms (ADR-4)"
# The probe's form logs what it hears: `form <kind> #<target> <value>`.
check "a label acts for the control its for= names" '#29 input#name value="" (focused)' \
  $HOST $PROBE --click "1:28" --frames 2
check "a label around a checkbox flips it" 'form change #31 "true"' \
  $HOST $PROBE --click "1:32" --frames 2
check "a select picks by value and says so" 'form change #33 "g"' \
  $HOST $PROBE --type "1:33=g" --frames 2
check "a range takes its value" 'form input #40 "8"' \
  $HOST $PROBE --type "1:40=8" --frames 2
check "a button in a form submits it" 'form submit #41' \
  $HOST $PROBE --click "1:42" --frames 2
check_absent "a disabled control answers nothing" '[wasm log] form' \
  $HOST $PROBE --click "1:43" --type "1:43=x" --frames 2
check "a select is drawn with its choice" '[paint] select #33' \
  $HOST $PROBE --paint --frames 1

echo
echo "==> markup (ADR-4)"
MARKUP='<div class="card"><h2>Hi &amp; bye</h2><p>One<br>two</p><img src=3/><input type=checkbox checked><!-- c --><script>evil()</script></div>'
check "set-markup builds elements, text and entities" '#48 "Hi & bye"' \
  $HOST $PROBE --frames 1 --type "0:45=$MARKUP"
check "void and self-closed elements need no closing tag" '#54 input [type="checkbox"] [x]' \
  $HOST $PROBE --frames 1 --type "0:45=$MARKUP"
check_absent "a script in markup is dropped, never run" '"evil()"' \
  $HOST $PROBE --frames 1 --type "0:45=$MARKUP"
check "markup it cannot read changes nothing and says where" 'markup: `</div>` closes `<p>`, at line 1, column 13' \
  $HOST $PROBE --frames 1 --type '0:45=<div><p>open</div>'

echo
echo "==> rates, the first real app"
# Declined, so cbu.uz is never contacted: the suite needs no internet.
check "rates asks before reaching the bank" "[asks] rates wants to connect to https://cbu.uz" \
  $HOST rates --frames 2
check "and, refused, says why it has nothing" "the user did not allow connecting to https://cbu.uz" \
  $HOST rates --frames 3
check "it speaks the user's language" '"Курсы валют"' \
  bash -c "LC_ALL= LC_MESSAGES= LANG=ru_RU.UTF-8 $HOST rates --frames 1"

echo
echo "==> what an app may know"
check "an app learns the user's language and region" "locale uz-UZ" \
  bash -c "LC_ALL= LC_MESSAGES= LANG=uz_UZ.UTF-8 $HOST mini-apps/fetcher/fetcher.component.wasm --frames 1"
check "and a default when the system does not say" "locale en-US" \
  bash -c "LC_ALL= LC_MESSAGES= LANG=C $HOST mini-apps/fetcher/fetcher.component.wasm --frames 1"

echo
echo "==> frames on request"
# The window draws only when something happens; an app showing time passing
# has to ask. One that asks for nothing must not be given frames anyway.
check "a guest can ask for its next frame" "[next frame in 1000 ms]" \
  $HOST hello --frames 1
check_absent "a guest that asks for nothing is given no frames" "[next frame" \
  $HOST counter --frames 2

echo
echo "==> scale"
# 500 apps in the registry: every tile is drawn every frame, and the whole lot
# has to stay inside a frame's output limits. How fast is reported, not judged
# here — timings depend on the machine (docs/ARCHITECTURE.md §1 has numbers).
BIG=$(mktemp -d)
{
  printf '{"apps":['
  for n in $(seq 1 500); do
    [[ $n -gt 1 ]] && printf ','
    printf '{"name":"app-%03d","source":"mini-apps/hello/hello.component.wasm","description":"Sample entry %d."}' "$n" "$n"
  done
  printf ']}\n'
} > "$BIG/apps.json"
check "a store of 500 apps draws them all within a frame's limits" '"500 apps"' \
  $HOST --registry "$(native "$BIG/apps.json")" store --frames 2 --timing
rm -rf "$BIG"
# Ten thousand nodes: two thousand rows of a list, built from markup the
# probe makes itself (`rows:N`), styled and laid out — the last row gets its
# box like the first.
check "a page of ten thousand nodes is styled and laid out" '"Item 1999" @' \
  $HOST $PROBE --frames 1 --boxes --timing --type "0:2=li { display: flex; gap: 8px }" --type "0:45=rows:2000"

echo
echo "==> making an app"
# What a developer outside this repository does: nimadir new, build, run.
if rustup target list --installed 2>/dev/null | grep -q '^wasm32-wasip2$'; then
  MADE=$(mktemp -d)
  TOOL="$PWD/target/debug/nimadir"
  check "nimadir new makes a project" "made demo/" \
    bash -c "cd '$MADE' && '$TOOL' new demo"
  check "with its own copy of this shell's WIT" "" \
    cmp "$MADE/demo/wit/world.wit" wit/world.wit
  check "nimadir build makes its component, manifest inside" "manifest  demo 0.1.0" \
    bash -c "cd '$MADE/demo' && '$TOOL' build"
  check "which the shell runs" '"Tapped 0 times."' \
    $HOST "$(native "$MADE/demo/demo.component.wasm")" --frames 1
  check "nimadir check says it will run" "runs on this shell" \
    "$TOOL" check "$MADE/demo/demo.component.wasm"
  check_fails "and says why one will not" "needs a newer nimadir" \
    "$TOOL" check mini-apps/future/future.component.wasm
  rm -rf "$MADE"
else
  skip "making an app (no wasm32-wasip2: rustup target add wasm32-wasip2)"
fi

echo
echo "==> address classification"
check "a local .wasm is a component" "loaded $COUNTER_BYTES bytes" \
  $HOST mini-apps/counter/counter.component.wasm --frames 1
check_fails "a local file that is neither is refused, without a network call" \
  "expected a .wasm component or an .html page" $HOST README.md
# Named .wasm but not one: classification lets it through, the parser stops it.
BOGUS=$(mktemp -d)/bogus.wasm
printf 'this is not webassembly\n' > "$BOGUS"
check_fails "a file named .wasm that is not one fails at parse, not at classification" \
  "not a WebAssembly component" $HOST "$BOGUS"
rm -rf "$(dirname "$BOGUS")"

echo
echo "==> sandbox limits"
# A guest is untrusted code; these are the two ways it could take the shell
# down with it. Each must end in the guest being stopped, with the limit named.
RUNAWAY=mini-apps/runaway/runaway.component.wasm
check_fails "a guest that never returns from a frame is stopped" "did not finish a frame within 1s" \
  $HOST $RUNAWAY --script "1:0" --frames 3
check_fails "a guest that grows its memory without end is stopped" "more than 512 MiB of memory" \
  $HOST $RUNAWAY --script "1:1" --frames 3
# The guest's memory cap does not cover what it hands the host: each label is a
# copy in the host's own memory. Bounded per frame, or one buffer sent in a loop
# fills the host.
check_fails "a guest that floods the host with UI is stopped" "in one frame and was stopped" \
  $HOST $RUNAWAY --script "1:2" --frames 3
check "and a guest that behaves is untouched by either" "Eat memory" \
  $HOST $RUNAWAY --frames 3
# The element tree outlives the frame, so it is bounded as a whole.
check_fails "a guest that builds nodes without end is stopped" "more than 65536 nodes" \
  $HOST $RUNAWAY --script "1:6" --frames 3
check_fails "a guest that nests its page too deep is stopped" "more than 256 levels deep" \
  $HOST $RUNAWAY --script "1:7" --frames 3
check_fails "a guest that fills its page with text is stopped" "more than 32 MiB of text" \
  $HOST $RUNAWAY --script "1:8" --frames 3
# Every host function, 40 calls a frame for 300 frames, with the worst inputs
# the fixture can think of. The guest logs NULs, which bash would warn about.
# The element tree too, its page styled, laid out and painted every frame.
check "the host answers every function's worst inputs without falling over" "fuzz frame 299, still here" \
  bash -c "set -o pipefail; $HOST mini-apps/fuzz/fuzz.component.wasm --frames 300 --styles --boxes --paint | tr -d '\\000'"
# The fixture's WIT must be this one, a minor version on, plus one function.
if command -v python3 >/dev/null 2>&1; then
  check "the future fixture's WIT is wit/world.wit one minor version on" "in step" \
    python3 -I tools/sync-fixture-wit.py --check
fi
# Built against a newer WIT: one function this host does not have.
check_fails "an app built for a newer host says it needs one" "needs a newer nimadir" \
  $HOST mini-apps/future/future.component.wasm --frames 1
check_fails "and names what it is missing" "from-the-future" \
  $HOST mini-apps/future/future.component.wasm --frames 1

echo
echo "==> address bar guessing"
# The guess is last: a registry name and a real file both beat it.
check "a registry name wins over any guess" "mini-apps/counter/counter.component.wasm" \
  $HOST counter --frames 1
# Observed through the address it tried, on a port where nothing listens: the
# guess is what is under test, and a real host would make the suite depend on
# the internet (and on whatever proxy stands between it and the machine).
check_fails "host-shaped text gets https://" "could not open https://localhost:9" \
  $HOST localhost:9
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
  # Sparse: 65 MiB by its length, no disk behind it. Only the size is tested.
  # Python rather than `truncate`, which macOS does not ship.
  python3 -c "import sys; open(sys.argv[1], 'wb').truncate(65 << 20)" "$FIXTURES/huge.wasm"
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
    check "application/wasm is a component" "loaded $COUNTER_BYTES bytes" \
      $HOST "$B/counter.component.wasm" --frames 1
    check "octet-stream with no extension: the magic number settles it" "loaded $COUNTER_BYTES bytes" \
      $HOST "$B/mystery" --frames 1
    check "text/html routes to the web page module" "is a web page" \
      $HOST "$B/page.html"
    check_fails "text/plain is neither, and says which it got" "Content-Type: text/plain" \
      $HOST "$B/notes.txt"
    check_fails "a download over the size cap is refused" "larger than 64 MiB" \
      $HOST "$B/huge.wasm"
    check "a registry can be served over HTTP" "served" \
      $HOST --registry "$B/catalogue.json" --list
    # The store lists whatever registry the host was given, not a compiled-in one.
    check "a served registry is what the store lists" '"1 apps"' \
      $HOST --registry "$B/catalogue.json" mini-apps/store/store.component.wasm --frames 1
    check "and its entries are the served ones" "served" \
      $HOST --registry "$B/catalogue.json" mini-apps/store/store.component.wasm --frames 1
    # Pinned and remote: fetched once, then opened with no server at all —
    # the second registry points at a port where nothing listens.
    printf '[{"name":"pinned","source":"%s/counter.component.wasm","sha256":"%s"}]\n' \
      "$B" "$COUNTER_SHA" > "$FIXTURES/pinned.json"
    printf '[{"name":"pinned","source":"http://127.0.0.1:9/counter.component.wasm","sha256":"%s"}]\n' \
      "$COUNTER_SHA" > "$FIXTURES/offline.json"
    check "a pinned remote component is fetched and checked" "loaded $COUNTER_BYTES bytes of wasm from $B" \
      $HOST --registry "$B/pinned.json" pinned --frames 1
    check "and opens again from the cache, with no network" "(from the cache)" \
      $HOST --registry "$(native "$FIXTURES/offline.json")" pinned --frames 1

    # The hosted registry, published as the Pages workflow does, and found by a
    # shell that has none of its own.
    python3 -I tools/publish_registry.py "$B/registry" "$FIXTURES/registry" >/dev/null
    check "the published registry pins every component it lists" '"sha256"' \
      cat "$FIXTURES/registry/apps.json"
    HOSTED_AT=$(mktemp -d)
    check "a shell with no registry of its own uses the hosted one" "$B/registry/hello.component.wasm" \
      bash -c "cd '$HOSTED_AT' && NIMADIR_HOSTED_REGISTRY='$B/registry/apps.json' '$ABS_HOST' --list"
    check "its store lists the hosted apps" '"hello"' \
      bash -c "cd '$HOSTED_AT' && NIMADIR_HOSTED_REGISTRY='$B/registry/apps.json' '$ABS_HOST' store --frames 1"
    check "and a hosted app opens, pinned" "Hello from a *different* mini-app!" \
      bash -c "cd '$HOSTED_AT' && NIMADIR_HOSTED_REGISTRY='$B/registry/apps.json' '$ABS_HOST' hello --frames 1"
    # With the server gone, the copy kept from the last fetch stands in.
    kill "$SRV" 2>/dev/null; wait "$SRV" 2>/dev/null; SRV=""
    check "with the network gone, the saved copy of the registry is used" "(saved copy)" \
      bash -c "cd '$HOSTED_AT' && NIMADIR_HOSTED_REGISTRY='$B/registry/apps.json' '$ABS_HOST' --list"
    rm -rf "$HOSTED_AT"

    # Network for apps (ADR-3): declared in the manifest, allowed by the user.
    # The server is gone by now (the saved-copy case stopped it), so start
    # another on the next port.
    PORT2=$((PORT + 1))
    (cd "$FIXTURES" && exec python3 -m http.server $PORT2 --bind 127.0.0.1) >/dev/null 2>&1 &
    SRV=$!
    B2="http://127.0.0.1:$PORT2"
    for _ in $(seq 1 40); do
      curl -sf --noproxy '*' -o /dev/null "$B2/page.html" && break
      sleep 0.25
    done
    # The suite's server is on this machine, which apps may not reach unless
    # this says they may.
    export NIMADIR_ALLOW_LOCAL_NETWORK=1
    NET=$(mktemp -d)
    $PACK keygen "$NET/key" >/dev/null
    printf '{"name":"fetcher","version":"1.0.0","permissions":[{"network":["%s"]}]}\n' "$B2" > "$NET/m.json"
    $PACK pack mini-apps/fetcher/fetcher.component.wasm "$NET/m.json" "$NET/signed.wasm" --key "$NET/key" >/dev/null
    $PACK pack mini-apps/fetcher/fetcher.component.wasm "$NET/m.json" "$NET/unsigned.wasm" >/dev/null
    FETCH=(--input "1:0=$B2/page.html" --script "2:0" --frames 4)
    check "an app's first request to an origin asks the user" "wants to connect to $B2" \
      $HOST "$(native "$NET/unsigned.wasm")" "${FETCH[@]}"
    check "without a yes it gets nothing" "the user did not allow connecting to $B2" \
      $HOST "$(native "$NET/unsigned.wasm")" "${FETCH[@]}"
    check "with one, it gets its answer" "status 200" \
      $HOST "$(native "$NET/signed.wasm")" "${FETCH[@]}" --confirm
    check_absent "and a signed app is not asked again" "wants to connect" \
      $HOST "$(native "$NET/signed.wasm")" "${FETCH[@]}"
    check "an origin its manifest does not list is refused at once" "is not among the origins" \
      $HOST "$(native "$NET/signed.wasm")" --input "1:0=http://127.0.0.1:9/x" --script "2:0" --frames 3
    # python's server answers a directory without its slash with a 301.
    check "a redirect is handed back to the app, not followed" "status 301" \
      $HOST "$(native "$NET/signed.wasm")" --input "1:0=$B2/registry" --script "2:0" --frames 4
    check "an app with no manifest reaches nothing" "declares no network access" \
      $HOST mini-apps/fetcher/fetcher.component.wasm "${FETCH[@]}"
    check "and without leave, no app reaches this machine or its network" "which apps may not reach" \
      env -u NIMADIR_ALLOW_LOCAL_NETWORK $HOST "$(native "$NET/signed.wasm")" "${FETCH[@]}"
    unset NIMADIR_ALLOW_LOCAL_NETWORK
    rm -rf "$NET"

    # Asked only when asked for; here, against releases the suite serves.
    printf '{"tag_name":"v9.9.9","html_url":"https://example.org/releases/9.9.9"}\n' > "$FIXTURES/newer.json"
    printf '{"tag_name":"v0.0.1","html_url":""}\n' > "$FIXTURES/older.json"
    printf '{"tag_name":"v1.0.0-alpha.3","html_url":""}\n' > "$FIXTURES/alpha.json"
    printf '{"tag_name":"v1.0.0-beta.2","html_url":"https://example.org/releases/beta.2"}\n' > "$FIXTURES/next.json"
    check "a newer release is reported, with where to get it" "nimadir 9.9.9 is out: https://example.org/releases/9.9.9" \
      bash -c "NIMADIR_UPDATE_URL='$B2/newer.json' $HOST --check-update"
    check "an older one is not" "is the newest" \
      bash -c "NIMADIR_UPDATE_URL='$B2/older.json' $HOST --check-update"
    check "an earlier pre-release is older than this one" "is the newest" \
      bash -c "NIMADIR_UPDATE_URL='$B2/alpha.json' $HOST --check-update"
    check "and a later one is newer" "nimadir 1.0.0-beta.2 is out" \
      bash -c "NIMADIR_UPDATE_URL='$B2/next.json' $HOST --check-update"
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
# The feature builds above leave a windowed binary at $HOST, which opens a
# window instead of running headless. Leave the headless one behind, so a case
# copied out of this file and run by hand does what it does in here.
cargo build -q -p host

echo
echo "-----------------------------------------------"
printf '  '; green "$PASS passed"
[[ $SKIP -gt 0 ]] && { printf ', '; dim "$SKIP skipped"; }
if [[ $FAIL -gt 0 ]]; then printf ', '; red "$FAIL failed"; echo; echo; exit 1; fi
echo; echo

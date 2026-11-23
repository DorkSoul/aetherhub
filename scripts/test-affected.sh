#!/usr/bin/env bash
# Runs only the checks that cover what changed, for quick feedback while working.
# The complete suite (every card scenario, smoke --all, every test) is scripts/test-full.sh: run that before a release.
#
# Usage: scripts/test-affected.sh [--since <git ref>]
#   default: uncommitted changes against HEAD, or the last commit when the working tree is clean.
#
# What runs:
#   card files (cards/<X>/<Name>.json, cards/tests/<Name>.json)  compile, scenarios and random play of those cards
#   engine Rust code                                              fast build, the engine's own tests, and the scenarios
#                                                                 of the cards that use the DSL types / rules areas the
#                                                                 change touches (plus the basic cards in cards/core)
#   TypeScript (packages/*, e2e/*)                                typecheck and unit tests; browser tests when the
#                                                                 client, server or e2e changed
# Card runs go through scripts/safe-run.sh (memory and time caps).
set -uo pipefail

cd "$(dirname "$0")/.."
ROOT=$PWD
SINCE=""
if [ "${1:-}" = "--since" ] && [ -n "${2:-}" ]; then
  SINCE=$2
fi

if [ -n "$SINCE" ]; then
  BASE=$SINCE
elif [ -n "$(git status --porcelain --untracked-files=normal)" ]; then
  BASE=HEAD
else
  BASE=HEAD~1
fi

CHANGED=$( { git diff --name-only "$BASE" --; git ls-files --others --exclude-standard; } | sort -u)
if [ -z "$CHANGED" ]; then
  echo "Nothing changed since $BASE."
  exit 0
fi
echo "Changes since $BASE: $(echo "$CHANGED" | wc -l) file(s)"

FAILED=0
fail() { echo "FAILED: $1"; FAILED=1; }
section() { echo; echo "=== $1"; }

has() { echo "$CHANGED" | grep -qE "$1"; }

ENGINE_CHANGED=0
has '^engine/(crates/|Cargo\.)' && ENGINE_CHANGED=1
CARDS_CHANGED=0
has '^cards/' && CARDS_CHANGED=1
TS_CHANGED=0
has '^(packages/|e2e/|package(-lock)?\.json|tsconfig|vitest|playwright)' && TS_CHANGED=1

# ---------------------------------------------------------------- engine build
if [ $ENGINE_CHANGED = 1 ] || [ $CARDS_CHANGED = 1 ]; then
  section "Engine build (fast profile)"
  if ! (cd engine && cargo build --profile fast 2>&1 | grep -E "^error" -A8; exit "${PIPESTATUS[0]}"); then
    fail "engine build"
    echo; echo "The engine doesn't build; stopping here."
    exit 1
  fi
fi
BIN=$ROOT/engine/target/fast

# ---------------------------------------------------------------- which cards to test
CARD_LIST=$(CHANGED="$CHANGED" BASE="$BASE" ENGINE_CHANGED=$ENGINE_CHANGED python3 - <<'EOF'
import glob, json, os, re, subprocess
changed = os.environ["CHANGED"].split("\n")
names = set()
# card definitions and scenario files that changed
for f in changed:
    m = re.match(r"cards/(?:tests/)?(?:[^/]+/)?([^/]+)\.json$", f)
    if m and not f.startswith(("cards/schema", "cards/README")):
        names.add(m.group(1))
if os.environ["ENGINE_CHANGED"] == "1":
    # DSL names (CamelCase identifiers) in the changed engine lines, matched against the "type" values cards use
    diff = subprocess.run(["git", "diff", "-U0", os.environ["BASE"], "--", "engine/crates"], capture_output=True, text=True).stdout
    # new (untracked) engine files count in full
    for f in changed:
        if f.startswith("engine/crates") and f.endswith(".rs") and os.path.exists(f):
            if subprocess.run(["git", "ls-files", "--error-unmatch", f], capture_output=True).returncode != 0:
                diff += "".join("+" + l + "\n" for l in open(f, encoding="utf-8").read().split("\n"))
    words = set()
    for line in diff.split("\n"):
        if line.startswith(("+", "-")) and not line.startswith(("+++", "---")):
            words.update(re.findall(r"\b(?:Effect|Cost|Trigger|Condition|StaticEffect|Ability|ChooseKind|Sel|Amount|Replace|Keyword)::([A-Z][A-Za-z0-9]+)", line))
            words.update(re.findall(r"\b([A-Z][a-z]+(?:[A-Z][a-z0-9]+)+)\b", line))
    # rules areas: an engine file -> words that mark the cards exercising it
    areas = {
        "combat.rs": ["Attacks", "Blocks", "MustAttack", "Goad", "CantBlock", "CantAttack", "Menace", "FirstStrike", "DoubleStrike", "Trample", "Banding"],
        "damage.rs": ["DealDamage", "Prevent", "Damage", "Lifelink", "Deathtouch", "Infect", "Wither"],
        "pay.rs": ["AddMana", "Mana", "Convoke", "Delve", "Improvise", "CumulativeUpkeep"],
        "cast.rs": ["AlternativeCost", "AdditionalCost", "Kicker", "Flashback", "Splice", "Buyback", "CostChange"],
        "layers.rs": ["Modify", "SetBasePT", "GrantKeyword", "AddType", "SetColors", "CopyPermanent", "GrantAbility"],
        "replace.rs": ["Replace", "EntersTapped", "EntersWithCounters", "EntersUnderOpponent"],
        "triggers.rs": ["Triggered", "Delayed"],
        "sba.rs": ["Indestructible", "Legendary", "Planeswalker", "Battle", "Aura"],
        "zones.rs": ["Move", "Exile", "Bounce", "Siege", "Aura"],
        "planechase.rs": ["Planechase", "Plane"],
    }
    for f in changed:
        base = os.path.basename(f)
        if f.startswith("engine/crates") and base in areas:
            words.update(areas[base])
    words = {w for w in words if len(w) > 3}
    if words:
        pat = re.compile(r'"(?:type|subtypes|keywords)"\s*:\s*(?:\[[^\]]*)?"(' + "|".join(map(re.escape, sorted(words))) + r')"')
        for f in glob.glob("cards/[A-Z_ÉÓ]*/*.json") + glob.glob("cards/core/*.json"):
            try:
                t = open(f, encoding="utf-8").read()
            except OSError:
                continue
            if pat.search(t):
                names.add(os.path.basename(f)[:-5])
    # the basic cards always run when the engine changed
    for f in glob.glob("cards/core/*.json"):
        names.add(os.path.basename(f)[:-5])
print("\n".join(sorted(n for n in names if n)))
EOF
)

if [ -n "$CARD_LIST" ]; then
  COUNT=$(echo "$CARD_LIST" | wc -l)
  # card definition files that exist (deleted ones are skipped)
  DEF_FILES=$(echo "$CARD_LIST" | while read -r n; do ls cards/*/"$n".json 2>/dev/null | grep -v '^cards/tests/'; done)
  TESTED=$(echo "$CARD_LIST" | while read -r n; do [ -f "cards/tests/$n.json" ] && echo "$n"; done)

  if [ -n "$DEF_FILES" ]; then
    section "Compile $(echo "$DEF_FILES" | wc -l) card file(s)"
    # shellcheck disable=SC2086
    if OUT=$("$BIN/cards" check $DEF_FILES 2>&1); then echo "$OUT" | tail -3; else echo "$OUT" | tail -20; fail "cards check"; fi
  fi

  if [ -n "$TESTED" ]; then
    section "Scenarios of $(echo "$TESTED" | wc -l) card(s) (of $COUNT selected)"
    LIMIT=$(( 120 + $(echo "$TESTED" | wc -l) / 2 ))
    # shellcheck disable=SC2086
    OUT=$(bash scripts/safe-run.sh "$LIMIT" "$BIN/cards" test $TESTED 2>&1)
    echo "$OUT" | grep -v '^  subset' | grep -E 'FAIL|scenario\(s\)|killed|error' | tail -30
    echo "$OUT" | grep -qE ' 0 failed' || fail "card scenarios"
  fi

  if [ -n "$DEF_FILES" ] && [ "$(echo "$DEF_FILES" | wc -l)" -le 300 ]; then
    NAMES=$(echo "$DEF_FILES" | while read -r f; do python3 -c "import json,sys; print(json.load(open(sys.argv[1]))['name'])" "$f" 2>/dev/null; done)
    section "Random play of $(echo "$NAMES" | wc -l) card(s)"
    mapfile -t NAME_ARR <<< "$NAMES"
    OUT=$(bash scripts/safe-run.sh 300 "$BIN/smoke" "${NAME_ARR[@]}" 2>&1)
    echo "$OUT" | tail -5
    echo "$OUT" | grep -qE ' 0 failed' || fail "smoke"
  fi
fi

# ---------------------------------------------------------------- engine tests
if [ $ENGINE_CHANGED = 1 ]; then
  section "Engine tests (without the every-scenario test)"
  if ! (cd engine && cargo test --profile fast --workspace -- --skip all_card_scenarios_pass 2>&1 | grep -E "^test result|FAILED|panicked|^error" | sort | uniq -c | tail -20; exit "${PIPESTATUS[0]}"); then
    fail "engine tests"
  fi
  if has '^engine/crates/[^/]+/src/(view|decision|state|ids|types)\.rs'; then
    echo "Note: an engine type shared with the UI may have changed: run 'npm run rules:types' and commit packages/shared/src/rules."
  fi
fi

# ---------------------------------------------------------------- TypeScript
if [ $TS_CHANGED = 1 ] || [ $ENGINE_CHANGED = 1 ]; then
  section "Typecheck"
  npm run typecheck >/dev/null 2>&1 || { npm run typecheck 2>&1 | grep "error TS" | head -20; fail "typecheck"; }

  section "Unit tests"
  NATIVE=$ROOT/engine/target/fast/libmtg_node.so
  [ -f "$NATIVE" ] || NATIVE=$ROOT/engine/target/release/libmtg_node.so
  OUT=$(RULES_NATIVE=$NATIVE npx vitest run 2>&1)
  echo "$OUT" | grep -E "Test Files|Tests |FAIL" | head -20
  echo "$OUT" | grep -qE "Tests .* passed" && ! echo "$OUT" | grep -q "failed" || fail "unit tests"

  if has '^(packages/(client|server|shared)/|e2e/)'; then
    section "Browser tests"
    if npm run build >/dev/null 2>&1; then
      OUT=$(npx playwright test 2>&1)
      echo "$OUT" | tail -4
      echo "$OUT" | grep -qE "[0-9]+ passed" && ! echo "$OUT" | grep -qE "[0-9]+ failed" || fail "browser tests"
    else
      fail "build"
    fi
  fi
fi

echo
if [ $FAILED = 0 ]; then
  echo "All checks for the changed areas passed. Before a release, run scripts/test-full.sh."
else
  echo "Some checks failed (see above)."
fi
exit $FAILED

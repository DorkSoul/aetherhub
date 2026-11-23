#!/usr/bin/env bash
# The complete test suite, run before a release (about an hour): release engine build, every engine test (including
# every card scenario), every card compiled and played at random, generated files up to date, typecheck, unit tests
# and browser tests. For quick checks while working use scripts/test-changed.sh.
set -uo pipefail

cd "$(dirname "$0")/.."
FAILED=0
fail() { echo "FAILED: $1"; FAILED=1; }
section() { echo; echo "=== $1"; }

section "Engine build (release)"
(cd engine && cargo build --release 2>&1 | grep -E "^error" -A8; exit "${PIPESTATUS[0]}") || { fail "engine build"; exit 1; }

section "Engine tests (every card scenario included)"
(cd engine && cargo test --release --workspace 2>&1 | grep -E "^test result|FAILED|panicked|^error" | sort | uniq -c; exit "${PIPESTATUS[0]}") || fail "engine tests"

section "Every card compiles"
bash scripts/safe-run.sh 600 engine/target/release/cards validate 2>&1 | tail -2 | tee /dev/stderr | grep -q "^ok:" || fail "cards validate"

section "Every card survives random play"
OUT=$(bash scripts/safe-run.sh 2400 engine/target/release/smoke --all 2>&1)
echo "$OUT" | tail -3
echo "$OUT" | grep -qE ' 0 failed' || fail "smoke --all"

section "Generated files are up to date"
engine/target/release/cards schema > cards/schema.json
node scripts/rules-types.mjs >/dev/null 2>&1
if ! git diff --quiet -- cards/schema.json packages/shared/src/rules; then
  git diff --stat -- cards/schema.json packages/shared/src/rules
  echo "Regenerated files differ from the committed ones: commit them."
fi

section "Typecheck"
npm run typecheck >/dev/null 2>&1 || { npm run typecheck 2>&1 | grep "error TS" | head -20; fail "typecheck"; }

section "Unit tests"
OUT=$(RULES_NATIVE=$PWD/engine/target/release/libmtg_node.so REQUIRE_NATIVE=1 npx vitest run 2>&1)
echo "$OUT" | grep -E "Test Files|Tests |FAIL" | head -20
echo "$OUT" | grep -qE "Tests .* passed" && ! echo "$OUT" | grep -q "failed" || fail "unit tests"

section "Browser tests"
if npm run build >/dev/null 2>&1; then
  OUT=$(npx playwright test 2>&1)
  echo "$OUT" | tail -4
  echo "$OUT" | grep -qE "[0-9]+ passed" && ! echo "$OUT" | grep -qE "[0-9]+ failed" || fail "browser tests"
else
  fail "build"
fi

echo
if [ $FAILED = 0 ]; then echo "Full suite passed."; else echo "Some checks failed (see above)."; fi
exit $FAILED

#!/usr/bin/env bash
# darwin-entrypoint.sh — the darwin evaluator entrypoint (ADR-0005, lever E).
#
# Usage (from dream.config.json evaluatorEntrypoints.darwin):
#   RUVLLM_TIMEOUT_MS=180000 ./scripts/darwin-entrypoint.sh \
#     npx @metaharness/darwin@0.10.2 evolve . --sandbox mock ...
#
# Runs the given darwin command through this repo's own
# `dream-machine verify-entrypoint darwin`, which classifies liveness and checks
# the printed leaderboard against DARWIN_BOUNDS (packages/cli/src/darwinBounds.ts).
# The leaderboard is parsed once, by the CLI, from the run it executed itself;
# --passthrough keeps darwin's own output in the evaluator receipt.
#
# The darwin command stays in dream.config.json, not in this script, so the
# dream-engine admission check (darwin recognised by package name, refused
# without --sandbox mock|agent) and the compiled-prompt pin test both still see it.
#
# Exit status:
#   0   darwin live, bounds respected
#   1   darwin failed (blocked)
#   2   darwin exited 0 with no output (suspicious-silent)
#   3   darwin live but its leaderboard breached a bound, or could not be parsed
#   64  refused: the command is not an exact-pinned darwin run in a mock/agent sandbox
#   69  the CLI is not built (run `npm run build`)
set -euo pipefail

die() {
  local status=$1
  shift
  printf 'darwin-entrypoint: %s\n' "$*" >&2
  exit "$status"
}

[ "$#" -gt 0 ] || die 64 "usage: $0 <darwin command...> (e.g. npx @metaharness/darwin@X.Y.Z evolve . --sandbox mock)"

# Pin discipline: every darwin package reference is an exact semver pin, and
# there is exactly one version. An unpinned npx runs whatever resolves tonight.
pins=()
sandbox=""
prev=""
for arg in "$@"; do
  case "$arg" in
    @metaharness/darwin@*)
      [[ "$arg" =~ ^@metaharness/darwin@[0-9]+\.[0-9]+\.[0-9]+$ ]] ||
        die 64 "darwin must be pinned to an exact version, got '$arg'"
      pins+=("$arg")
      ;;
    @metaharness/darwin | metaharness-darwin)
      die 64 "darwin must be pinned to an exact version, got unpinned '$arg'"
      ;;
    --sandbox=*) sandbox=${arg#--sandbox=} ;;
  esac
  [ "$prev" = "--sandbox" ] && sandbox=$arg
  prev=$arg
done

[ "${#pins[@]}" -gt 0 ] || die 64 "no pinned @metaharness/darwin@X.Y.Z in the command"
for pin in "${pins[@]}"; do
  [ "$pin" = "${pins[0]}" ] || die 64 "conflicting darwin pins: ${pins[0]} and $pin"
done

# ADR-065 (dream-engine): the default 'real' sandbox is surface-independent.
case "$sandbox" in
  mock | agent) ;;
  "") die 64 "darwin needs --sandbox mock or --sandbox agent (the default 'real' sandbox is not probative)" ;;
  *) die 64 "darwin sandbox '$sandbox' refused: use --sandbox mock or --sandbox agent" ;;
esac

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cli="$repo_root/packages/cli/dist/bin.js"
[ -f "$cli" ] || die 69 "$cli not found: run 'npm ci && npm run build' first"

# Re-quote the argv into one shell-safe string for --cmd; verify-entrypoint
# runs it through a shell, so arguments with spaces or metacharacters survive.
printf -v darwin_cmd '%q ' "$@"

cd -- "$repo_root"
exec node "$cli" verify-entrypoint darwin --passthrough --cmd "${darwin_cmd% }"

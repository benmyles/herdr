#!/bin/sh
# installed by herdr
# managed by herdr; reinstalling or updating the integration overwrites this file.
# add custom hooks beside this file instead of editing it.
# HERDR_INTEGRATION_ID=crush
# HERDR_INTEGRATION_VERSION=1

# Crush fires this hook on SessionStart (fresh or resumed sessions). Herdr
# uses it for session identity only; crush state stays with screen detection.
# The hook runs inside crush's embedded POSIX shell, which guarantees the
# builtin `jq`; python3 is optional and only supplies a monotonic sequence.

action="${1:-}"
[ "$action" = "session" ] || exit 0
[ "${HERDR_ENV:-}" = "1" ] || exit 0
[ -n "${HERDR_SOCKET_PATH:-}" ] || exit 0
[ -n "${HERDR_PANE_ID:-}" ] || exit 0
bin="${HERDR_BIN_PATH:-herdr}"

session_id="$(jq -r '.session_id // empty' 2>/dev/null || true)"
[ -n "$session_id" ] || exit 0

args="pane report-agent-session $HERDR_PANE_ID --source herdr:crush --agent crush --agent-session-id $session_id"
if command -v python3 >/dev/null 2>&1; then
    seq="$(python3 -c 'import time; print(time.time_ns())' 2>/dev/null)" || seq=""
fi
if [ -n "${seq:-}" ]; then
    args="$args --seq $seq"
fi

# The command is assembled from herdr-owned ids (pane ids are tokens like
# w1:p2; session ids are short hex), so word splitting is deliberate and safe.
# shellcheck disable=SC2086
"$bin" $args >/dev/null 2>&1 || true

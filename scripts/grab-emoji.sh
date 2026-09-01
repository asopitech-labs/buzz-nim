#!/usr/bin/env bash
# grab-emoji.sh — Register custom Slack emoji in Nimino
#
# Looks up each emoji name in your Slack workspace and registers it in Nimino
# via `nimino emoji set`, making it available as :name: in the Nimino emoji picker.
#
# Usage:
#   SLACK_TOKEN=xoxp-... ./scripts/grab-emoji.sh [--name <nimino-name>] <emoji-name> [emoji-name ...]
#
# Options:
#   --name <nimino-name>  Override the shortcode used in Nimino (only valid with a single emoji)
#
# Env:
#   SLACK_TOKEN  — Slack user token (xoxp-...) with emoji:read scope
#
# Output:
#   name → registered as :name: in Nimino   on success
#   name → ERROR: reason                  on failure (script continues to next emoji)

set -euo pipefail

CACHE_FILE="${HOME}/.cache/slack-emoji-list.json"
CACHE_TTL=86400  # 24 hours in seconds

# ── Argument parsing ──────────────────────────────────────────────────────────

NIMINO_NAME=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --name)
      NIMINO_NAME="$2"
      shift 2
      ;;
    --)
      shift
      break
      ;;
    -*)
      echo "ERROR: Unknown option: $1" >&2
      exit 1
      ;;
    *)
      break
      ;;
  esac
done

# ── Preflight checks ──────────────────────────────────────────────────────────

if [[ $# -eq 0 ]]; then
  echo "Usage: SLACK_TOKEN=xoxp-... $0 [--name <nimino-name>] <emoji-name> [emoji-name ...]" >&2
  exit 1
fi

if [[ -n "$NIMINO_NAME" && $# -ne 1 ]]; then
  echo "ERROR: --name can only be used when specifying a single emoji" >&2
  exit 1
fi

if [[ -z "${SLACK_TOKEN:-}" ]]; then
  echo "ERROR: SLACK_TOKEN is not set. Export your xoxp- Slack token." >&2
  exit 1
fi

if ! command -v nimino &>/dev/null; then
  echo "ERROR: 'nimino' not found in PATH. Install the Nimino CLI and retry." >&2
  exit 1
fi

if ! command -v curl &>/dev/null; then
  echo "ERROR: 'curl' not found in PATH." >&2
  exit 1
fi

if ! command -v jq &>/dev/null; then
  echo "ERROR: 'jq' not found in PATH." >&2
  exit 1
fi

# ── Cache management ──────────────────────────────────────────────────────────

_cache_is_fresh() {
  [[ -f "$CACHE_FILE" ]] || return 1
  local mtime now age
  # macOS stat uses -f %m; GNU stat uses -c %Y
  if stat --version &>/dev/null 2>&1; then
    mtime=$(stat -c %Y "$CACHE_FILE")
  else
    mtime=$(stat -f %m "$CACHE_FILE")
  fi
  now=$(date +%s)
  age=$(( now - mtime ))
  (( age < CACHE_TTL ))
}

_refresh_cache() {
  mkdir -p "$(dirname "$CACHE_FILE")"
  local response
  response=$(curl -sf \
    -H "Authorization: Bearer ${SLACK_TOKEN}" \
    "https://slack.com/api/emoji.list") || {
    echo "ERROR: network failure fetching emoji list from Slack" >&2
    return 1
  }

  local ok
  ok=$(echo "$response" | jq -r '.ok')
  if [[ "$ok" != "true" ]]; then
    local err
    err=$(echo "$response" | jq -r '.error // "unknown error"')
    echo "ERROR: Slack API returned error: $err" >&2
    return 1
  fi

  echo "$response" > "$CACHE_FILE"
}

if ! _cache_is_fresh; then
  _refresh_cache || exit 1
fi

# ── Emoji resolution ──────────────────────────────────────────────────────────

# Returns the URL for an emoji name, resolving one level of aliasing.
# Prints the URL to stdout; returns 1 if not found.
_resolve_url() {
  local name="$1"
  local value
  value=$(jq -r --arg n "$name" '.emoji[$n] // empty' "$CACHE_FILE")

  if [[ -z "$value" ]]; then
    return 1
  fi

  if [[ "$value" == alias:* ]]; then
    local target="${value#alias:}"
    value=$(jq -r --arg n "$target" '.emoji[$n] // empty' "$CACHE_FILE")
    if [[ -z "$value" ]]; then
      return 1
    fi
  fi

  echo "$value"
}

# ── Per-emoji processing ──────────────────────────────────────────────────────

for emoji_name in "$@"; do
  # Use --name override if provided, otherwise use the Slack emoji name
  nimino_shortcode="${NIMINO_NAME:-$emoji_name}"

  # Resolve URL
  emoji_url=$(_resolve_url "$emoji_name") || {
    echo "${emoji_name} → ERROR: emoji not found in workspace"
    continue
  }

  # Register in Nimino
  set_output=$(nimino emoji set --shortcode "$nimino_shortcode" --url "$emoji_url" 2>&1) || {
    echo "${emoji_name} → ERROR: nimino emoji set failed — ${set_output}"
    continue
  }

  echo "${emoji_name} → registered as :${nimino_shortcode}: in Nimino"
done

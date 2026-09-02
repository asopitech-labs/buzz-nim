#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export HOME="$tmp/home"
export NIMINO_TEST_PLATFORM=Darwin
mkdir -p "$HOME/Library/Application Support/com.asopitech.nimino.dev.example"
mkdir -p "$HOME/Library/Application Support/com.asopitech.nimino.dev.other"
mkdir -p "$HOME/Library/Application Support/com.asopitech.nimino"
mkdir -p "$HOME/.nimino-dev"
touch "$HOME/.nimino-dev/keep"
mkdir -p "$tmp/bin"
cat > "$tmp/bin/security" <<'MOCK'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$HOME/security-calls"
exit 1
MOCK
chmod +x "$tmp/bin/security"
export PATH="$tmp/bin:$PATH"

"$repo_root/scripts/reset-desktop-standalone-state.sh" \
    com.asopitech.nimino.dev.example nimino-desktop-dev.example

[[ ! -e "$HOME/Library/Application Support/com.asopitech.nimino.dev.example" ]]
[[ -d "$HOME/Library/Application Support/com.asopitech.nimino.dev.other" ]]
[[ -d "$HOME/Library/Application Support/com.asopitech.nimino" ]]
[[ -f "$HOME/.nimino-dev/keep" ]]
grep -Fx -- "delete-generic-password -s nimino-desktop-dev.example" "$HOME/security-calls" >/dev/null

if "$repo_root/scripts/reset-desktop-standalone-state.sh" \
    com.asopitech.nimino nimino-desktop >/dev/null 2>&1; then
    echo "expected production scope guard to reject reset" >&2
    exit 1
fi

echo "standalone desktop reset scope test passed"

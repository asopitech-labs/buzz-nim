#!/usr/bin/env bash

set -euo pipefail

fail() {
    echo "error: $*" >&2
    exit 1
}

[[ $# -ge 1 ]] || fail "usage: nimino-wsl-service.sh <install|update|restart|uninstall>"
COMMAND="$1"
shift

DATA_HOME="${XDG_DATA_HOME:-${HOME:?HOME is required}/.local/share}"
STATE_HOME="${XDG_STATE_HOME:-${HOME}/.local/state}"
CONFIG_HOME="${XDG_CONFIG_HOME:-${HOME}/.config}"
BIN_HOME="${XDG_BIN_HOME:-${HOME}/.local/bin}"
APP_ROOT="${DATA_HOME}/nimino"
RELEASES="${APP_ROOT}/service-releases"
CURRENT="${APP_ROOT}/current"
DATA_DIR="${APP_ROOT}/data"
STATE_DIR="${STATE_HOME}/nimino"
UNIT_DIR="${CONFIG_HOME}/systemd/user"
UNIT_PATH="${UNIT_DIR}/nimino-relay.service"
SYSTEMCTL="${NIMINO_SYSTEMCTL:-systemctl}"
STAGING=""
TOOL_NAMES=(nimino nimino-acp nimino-agent nimino-core-worker nimino-data-ops nimino-dev-mcp nimino-relay)
CHECKSUM_PATHS=(
    bin/nimino
    bin/nimino-acp
    bin/nimino-agent
    bin/nimino-core-worker
    bin/nimino-data-ops
    bin/nimino-dev-mcp
    bin/nimino-relay
    nimino-wsl-bundle.json
    release-set-id
    release-set.json
)

cleanup_staging() {
    [[ -z "$STAGING" ]] || rm -rf -- "$STAGING"
}
trap cleanup_staging EXIT

case "$APP_ROOT:$STATE_DIR:$UNIT_PATH:$BIN_HOME" in
    *$'\n'*|*$'\r'*|*'"'*|*'\'*|*'%'*) fail "WSL lifecycle paths contain unsafe systemd characters" ;;
esac
[[ "$APP_ROOT" == */nimino && "$STATE_DIR" == */nimino ]] || fail "unsafe lifecycle root"
command -v "$SYSTEMCTL" >/dev/null 2>&1 || fail "systemctl is unavailable"
command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is unavailable"

systemctl_user() {
    "$SYSTEMCTL" --user "$@"
}

switch_current() {
    local target="$1" temporary="${CURRENT}.new"
    rm -f -- "$temporary"
    ln -s "$target" "$temporary"
    mv -Tf -- "$temporary" "$CURRENT"
}

write_unit() {
    local temporary="${UNIT_PATH}.new"
    mkdir -p "$UNIT_DIR"
    {
        printf '%s\n' '[Unit]'
        printf '%s\n' 'Description=Nimino relay'
        printf '%s\n' 'After=network-online.target'
        printf '%s\n' 'Wants=network-online.target'
        printf '\n%s\n' '[Service]'
        printf 'ExecStart="%s"\n' "${CURRENT}/bin/nimino-relay"
        printf 'Environment="NIMINO_DATA_DIR=%s"\n' "$DATA_DIR"
        printf 'Environment="NIMINO_BOUNDARY_WORKER=%s"\n' "${CURRENT}/bin/nimino-core-worker"
        printf '%s\n' 'Restart=on-failure' 'RestartSec=2s' 'KillMode=control-group'
        printf '\n%s\n' '[Install]'
        printf '%s\n' 'WantedBy=default.target'
    } > "$temporary"
    chmod 0644 "$temporary"
    mv -Tf -- "$temporary" "$UNIT_PATH"
}

parse_release() {
    RELEASE_SET_ID=""
    BUNDLE=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --release-set-id)
                [[ $# -ge 2 ]] || fail "--release-set-id requires a value"
                RELEASE_SET_ID="$2"
                shift 2
                ;;
            --bundle)
                [[ $# -ge 2 ]] || fail "--bundle requires a value"
                BUNDLE="$2"
                shift 2
                ;;
            *) fail "unknown argument: $1" ;;
        esac
    done
    [[ "$RELEASE_SET_ID" =~ ^[0-9a-f]{64}$ ]] || fail "release-set ID must be 64 lowercase hex characters"
    [[ -d "$BUNDLE" && ! -L "$BUNDLE" ]] || fail "WSL bundle directory is missing"
    local root_entries expected_entries actual_bins expected_bins bundle_id
    root_entries="$(find "$BUNDLE" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
    expected_entries=$'SHA256SUMS\nbin\nnimino-wsl-bundle.json\nrelease-set-id\nrelease-set.json'
    [[ "$root_entries" == "$expected_entries" ]] || fail "WSL bundle root inventory is invalid"
    actual_bins="$(find "$BUNDLE/bin" -mindepth 1 -maxdepth 1 -printf '%f\n' | sort)"
    expected_bins="$(printf '%s\n' "${TOOL_NAMES[@]}" | sort)"
    [[ "$actual_bins" == "$expected_bins" ]] || fail "WSL bundle binary inventory is invalid"
    for name in "${TOOL_NAMES[@]}"; do
        [[ -f "$BUNDLE/bin/$name" && ! -L "$BUNDLE/bin/$name" && -x "$BUNDLE/bin/$name" ]] ||
            fail "WSL bundle binary is invalid: $name"
    done
    for name in SHA256SUMS nimino-wsl-bundle.json release-set-id release-set.json; do
        [[ -f "$BUNDLE/$name" && ! -L "$BUNDLE/$name" ]] || fail "WSL bundle metadata is invalid: $name"
    done
    local -a checksum_paths=()
    while IFS= read -r line; do
        [[ "$line" =~ ^[0-9a-f]{64}[[:space:]][[:space:]]([a-z0-9./-]+)$ ]] || fail "WSL checksum inventory is malformed"
        checksum_paths+=("${BASH_REMATCH[1]}")
    done < "$BUNDLE/SHA256SUMS"
    [[ "${checksum_paths[*]}" == "${CHECKSUM_PATHS[*]}" ]] || fail "WSL checksum inventory is incomplete"
    (cd "$BUNDLE" && sha256sum --check --strict SHA256SUMS >/dev/null) || fail "WSL bundle checksum verification failed"
    bundle_id="$(<"$BUNDLE/release-set-id")"
    [[ "$bundle_id" == "$RELEASE_SET_ID" ]] || fail "WSL bundle release-set ID mismatch"
    grep -Fq "\"releaseSetId\": \"$RELEASE_SET_ID\"" "$BUNDLE/nimino-wsl-bundle.json" ||
        fail "WSL bundle provenance mismatch"
    grep -Fq "\"releaseSetId\": \"$RELEASE_SET_ID\"" "$BUNDLE/release-set.json" ||
        fail "release-set provenance mismatch"
}

check_tool_links() {
    local name link target
    for name in "${TOOL_NAMES[@]}"; do
        link="$BIN_HOME/$name"
        target="$CURRENT/bin/$name"
        if [[ -e "$link" || -L "$link" ]]; then
            [[ -L "$link" && "$(readlink "$link")" == "$target" ]] || fail "refusing to replace existing tool: $link"
        fi
    done
}

link_tools() {
    local name link target temporary
    mkdir -p "$BIN_HOME"
    for name in "${TOOL_NAMES[@]}"; do
        link="$BIN_HOME/$name"
        target="$CURRENT/bin/$name"
        temporary="${link}.new"
        rm -f -- "$temporary"
        ln -s "$target" "$temporary"
        mv -Tf -- "$temporary" "$link"
    done
}

remove_tool_links() {
    local name link target
    for name in "${TOOL_NAMES[@]}"; do
        link="$BIN_HOME/$name"
        target="$CURRENT/bin/$name"
        if [[ -L "$link" && "$(readlink "$link")" == "$target" ]]; then
            rm -f -- "$link"
        fi
    done
}

activate_release() {
    local mode="$1" release created=0 previous=""
    release="${RELEASES}/${RELEASE_SET_ID}"
    if [[ -L "$CURRENT" ]]; then
        previous="$(readlink "$CURRENT")"
        [[ "$previous" == "${RELEASES}/"* && -d "$previous" ]] || fail "current release link is invalid"
    fi
    [[ "$mode" != update || -n "$previous" ]] || fail "update requires an installed release"
    check_tool_links

    mkdir -p "$RELEASES" "$DATA_DIR" "$STATE_DIR"
    if [[ -d "$release" ]]; then
        for path in "${CHECKSUM_PATHS[@]}" SHA256SUMS; do
            cmp -s "$BUNDLE/$path" "$release/$path" || fail "release-set ID already has different content"
        done
    else
        STAGING="$(mktemp -d "${RELEASES}/.staging.XXXXXX")"
        mkdir -p "${STAGING}/bin"
        for name in "${TOOL_NAMES[@]}"; do
            install -m 0755 "$BUNDLE/bin/$name" "${STAGING}/bin/$name"
        done
        for name in SHA256SUMS nimino-wsl-bundle.json release-set-id release-set.json; do
            install -m 0644 "$BUNDLE/$name" "${STAGING}/$name"
        done
        mv -T -- "$STAGING" "$release"
        STAGING=""
        created=1
    fi

    write_unit
    switch_current "$release"
    link_tools
    if [[ "$mode" == install ]]; then
        if systemctl_user daemon-reload &&
            systemctl_user enable --now nimino-relay.service &&
            systemctl_user is-active --quiet nimino-relay.service; then
            return
        fi
    else
        if systemctl_user daemon-reload &&
            systemctl_user restart nimino-relay.service &&
            systemctl_user is-active --quiet nimino-relay.service; then
            return
        fi
    fi

    if [[ -n "$previous" ]]; then
        switch_current "$previous"
        systemctl_user restart nimino-relay.service
        systemctl_user is-active --quiet nimino-relay.service || fail "update failed and rollback could not restart the previous release"
        [[ "$created" == 0 ]] || rm -rf -- "$release"
        fail "update failed health check and was rolled back"
    fi
    remove_tool_links
    systemctl_user disable --now nimino-relay.service >/dev/null 2>&1 || true
    rm -f -- "$CURRENT" "$UNIT_PATH"
    [[ "$created" == 0 ]] || rm -rf -- "$release"
    systemctl_user daemon-reload >/dev/null 2>&1 || true
    rmdir "$DATA_DIR" "$RELEASES" "$APP_ROOT" "$STATE_DIR" >/dev/null 2>&1 || true
    fail "install failed health check"
}

case "$COMMAND" in
    install|update)
        parse_release "$@"
        activate_release "$COMMAND"
        ;;
    restart)
        [[ $# -eq 0 ]] || fail "restart takes no arguments"
        [[ -L "$CURRENT" && -f "$UNIT_PATH" ]] || fail "Nimino relay is not installed"
        systemctl_user restart nimino-relay.service
        systemctl_user is-active --quiet nimino-relay.service || fail "relay failed health check after restart"
        ;;
    uninstall)
        PURGE=0
        if [[ $# -eq 1 && "$1" == --purge-data ]]; then
            PURGE=1
        elif [[ $# -ne 0 ]]; then
            fail "uninstall accepts only --purge-data"
        fi
        systemctl_user disable --now nimino-relay.service >/dev/null 2>&1 || true
        remove_tool_links
        rm -f -- "$UNIT_PATH" "$CURRENT"
        rm -rf -- "$RELEASES"
        systemctl_user daemon-reload >/dev/null 2>&1 || true
        if [[ "$PURGE" == 1 ]]; then
            rm -rf -- "$APP_ROOT" "$STATE_DIR"
        fi
        ;;
    *) fail "unknown command: $COMMAND" ;;
esac

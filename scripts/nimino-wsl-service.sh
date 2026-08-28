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
APP_ROOT="${DATA_HOME}/nimino"
RELEASES="${APP_ROOT}/service-releases"
CURRENT="${APP_ROOT}/current"
DATA_DIR="${APP_ROOT}/data"
STATE_DIR="${STATE_HOME}/nimino"
UNIT_DIR="${CONFIG_HOME}/systemd/user"
UNIT_PATH="${UNIT_DIR}/nimino-relay.service"
SYSTEMCTL="${NIMINO_SYSTEMCTL:-systemctl}"

case "$APP_ROOT:$STATE_DIR:$UNIT_PATH" in
    *$'\n'*|*$'\r'*|*'"'*|*'\'*|*'%'*) fail "WSL lifecycle paths contain unsafe systemd characters" ;;
esac
[[ "$APP_ROOT" == */nimino && "$STATE_DIR" == */nimino ]] || fail "unsafe lifecycle root"
command -v "$SYSTEMCTL" >/dev/null 2>&1 || fail "systemctl is unavailable"

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
        printf '%s\n' 'Restart=on-failure' 'RestartSec=2s' 'KillMode=control-group'
        printf '\n%s\n' '[Install]'
        printf '%s\n' 'WantedBy=default.target'
    } > "$temporary"
    chmod 0644 "$temporary"
    mv -Tf -- "$temporary" "$UNIT_PATH"
}

parse_release() {
    RELEASE_SET_ID=""
    RELAY=""
    while [[ $# -gt 0 ]]; do
        case "$1" in
            --release-set-id)
                [[ $# -ge 2 ]] || fail "--release-set-id requires a value"
                RELEASE_SET_ID="$2"
                shift 2
                ;;
            --relay)
                [[ $# -ge 2 ]] || fail "--relay requires a value"
                RELAY="$2"
                shift 2
                ;;
            *) fail "unknown argument: $1" ;;
        esac
    done
    [[ "$RELEASE_SET_ID" =~ ^[0-9a-f]{64}$ ]] || fail "release-set ID must be 64 lowercase hex characters"
    [[ -f "$RELAY" && -x "$RELAY" ]] || fail "relay executable is missing"
}

activate_release() {
    local mode="$1" release created=0 previous=""
    release="${RELEASES}/${RELEASE_SET_ID}"
    if [[ -L "$CURRENT" ]]; then
        previous="$(readlink "$CURRENT")"
        [[ "$previous" == "${RELEASES}/"* && -d "$previous" ]] || fail "current release link is invalid"
    fi
    [[ "$mode" != update || -n "$previous" ]] || fail "update requires an installed release"

    mkdir -p "$RELEASES" "$DATA_DIR" "$STATE_DIR"
    if [[ -d "$release" ]]; then
        cmp -s "$RELAY" "${release}/bin/nimino-relay" || fail "release-set ID already has different content"
    else
        local staging
        staging="$(mktemp -d "${RELEASES}/.staging.XXXXXX")"
        mkdir -p "${staging}/bin"
        install -m 0755 "$RELAY" "${staging}/bin/nimino-relay"
        mv -T -- "$staging" "$release"
        created=1
    fi

    write_unit
    switch_current "$release"
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
    rm -f -- "$CURRENT"
    systemctl_user disable --now nimino-relay.service >/dev/null 2>&1 || true
    [[ "$created" == 0 ]] || rm -rf -- "$release"
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
        rm -f -- "$UNIT_PATH" "$CURRENT"
        rm -rf -- "$RELEASES"
        systemctl_user daemon-reload >/dev/null 2>&1 || true
        if [[ "$PURGE" == 1 ]]; then
            rm -rf -- "$APP_ROOT" "$STATE_DIR"
        fi
        ;;
    *) fail "unknown command: $COMMAND" ;;
esac

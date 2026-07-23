#!/usr/bin/env bash
#
# install-linux.sh — install skid-monitor-agent as a systemd service on Linux.
#
# Rationale: docs/agent-continuous-deployment.md lists a signed .deb/.rpm/tar.gz
# package as the eventual first-class artifact for Linux native installs, but
# that packaging pipeline does not exist yet ("Current Gaps"). This script is
# the practical stand-in: it builds skid-monitor-agent from source (or accepts
# a prebuilt binary), then installs it the way that doc describes — dedicated
# user, config/data directories, systemd unit — so it can be replaced by a real
# package later without changing the deployment shape.
#
# Usage:
#   sudo ./install-linux.sh [options]
#
# Common options:
#   --client-addr ADDR      SKID_MONITOR_CLIENT_ADDR for the default (Solo) exporter.
#                           Default: 127.0.0.1:9000. Ignored if --config is given.
#   --config PATH           Use a custom SKID_MONITOR_AGENT_CONFIG JSON file
#                           (e.g. a cloud exporter config) instead of the default
#                           env-var-only Solo setup.
#   --set-env KEY=VALUE     Add an extra line to the service environment file.
#                           Repeatable. Use this to inject secrets referenced by
#                           a config file's "client_secret_env" (never put the
#                           secret value itself in the config JSON).
#   --extra-group GROUP     Add the service user to an existing group (repeatable),
#                           e.g. --extra-group adm so the database-log receiver
#                           can read /var/log/postgresql/*.
#   --binary PATH           Use a prebuilt skid-monitor-agent binary instead of
#                           building from source with cargo.
#   --repo PATH             Path to a skid-monitor workspace checkout to build
#                           from. Default: auto-detected from this script's location.
#   --install-dir DIR       Binary install directory. Default: /usr/local/bin
#   --config-dir DIR        Config directory. Default: /etc/skid-monitor-agent
#   --data-dir DIR          State/data directory. Default: /var/lib/skid-monitor-agent
#   --user NAME             Service user/group name. Default: skid-monitor-agent
#   --no-start              Enable the service but do not start it now.
#   --force                 Overwrite an existing env file / config file / unit.
#   --uninstall             Stop, disable, and remove the service and binary.
#   --purge                 With --uninstall, also remove config/data dirs and the user.
#   -h, --help              Show this help.
#
# Examples:
#   sudo ./install-linux.sh --client-addr 127.0.0.1:9000
#   sudo ./install-linux.sh --config /etc/skid-monitor-agent/agent-cloud-config.json \
#        --set-env SKID_MONITOR_OIDC_CLIENT_SECRET=***
#   sudo ./install-linux.sh --uninstall --purge

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DEFAULT_REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"

SERVICE_NAME="skid-monitor-agent"
SERVICE_USER="skid-monitor-agent"
INSTALL_DIR="/usr/local/bin"
CONFIG_DIR="/etc/skid-monitor-agent"
DATA_DIR="/var/lib/skid-monitor-agent"
UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"

REPO_ROOT="${DEFAULT_REPO_ROOT}"
BINARY_PATH=""
CONFIG_SRC=""
CLIENT_ADDR="127.0.0.1:9000"
EXTRA_ENV_LINES=()
EXTRA_GROUPS=()
START_NOW=1
FORCE=0
DO_UNINSTALL=0
DO_PURGE=0

log()  { printf '[install-agent] %s\n' "$*"; }
warn() { printf '[install-agent] WARN: %s\n' "$*" >&2; }
die()  { printf '[install-agent] ERROR: %s\n' "$*" >&2; exit 1; }

usage() {
    sed -n '2,47p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

# ---------------------------------------------------------------------------
# Arg parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --client-addr) CLIENT_ADDR="$2"; shift 2 ;;
        --config) CONFIG_SRC="$2"; shift 2 ;;
        --set-env) EXTRA_ENV_LINES+=("$2"); shift 2 ;;
        --extra-group) EXTRA_GROUPS+=("$2"); shift 2 ;;
        --binary) BINARY_PATH="$2"; shift 2 ;;
        --repo) REPO_ROOT="$2"; shift 2 ;;
        --install-dir) INSTALL_DIR="$2"; shift 2 ;;
        --config-dir) CONFIG_DIR="$2"; shift 2 ;;
        --data-dir) DATA_DIR="$2"; shift 2 ;;
        --user) SERVICE_USER="$2"; shift 2 ;;
        --no-start) START_NOW=0; shift ;;
        --force) FORCE=1; shift ;;
        --uninstall) DO_UNINSTALL=1; shift ;;
        --purge) DO_PURGE=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) die "unknown argument: $1 (see --help)" ;;
    esac
done

UNIT_PATH="/etc/systemd/system/${SERVICE_NAME}.service"

# ---------------------------------------------------------------------------
# Preconditions
# ---------------------------------------------------------------------------
require_root() {
    [[ "$(id -u)" -eq 0 ]] || die "run this script as root (sudo ./install-linux.sh ...)"
}

require_linux_systemd() {
    [[ "$(uname -s)" == "Linux" ]] || die "this script only supports Linux"
    command -v systemctl >/dev/null 2>&1 || die "systemd (systemctl) not found; this script only supports systemd hosts"
}

# ---------------------------------------------------------------------------
# Uninstall path
# ---------------------------------------------------------------------------
uninstall() {
    log "uninstalling ${SERVICE_NAME}"
    if systemctl is-active --quiet "${SERVICE_NAME}" 2>/dev/null; then
        systemctl stop "${SERVICE_NAME}"
    fi
    if systemctl is-enabled --quiet "${SERVICE_NAME}" 2>/dev/null; then
        systemctl disable "${SERVICE_NAME}"
    fi
    rm -f "${UNIT_PATH}"
    systemctl daemon-reload
    rm -f "${INSTALL_DIR}/${SERVICE_NAME}"

    if [[ "${DO_PURGE}" -eq 1 ]]; then
        log "purging config/data directories and service user"
        rm -rf "${CONFIG_DIR}" "${DATA_DIR}"
        if id "${SERVICE_USER}" >/dev/null 2>&1; then
            userdel "${SERVICE_USER}" 2>/dev/null || warn "could not remove user ${SERVICE_USER}"
        fi
    else
        log "keeping ${CONFIG_DIR} and ${DATA_DIR} (pass --purge to remove them too)"
    fi
    log "uninstall complete"
}

# ---------------------------------------------------------------------------
# Build or locate the binary
# ---------------------------------------------------------------------------
build_binary() {
    [[ -f "${REPO_ROOT}/Cargo.toml" ]] || die "no Cargo.toml at ${REPO_ROOT}; pass --repo <path> or --binary <path>"
    grep -q '"skid-monitor-agent"' "${REPO_ROOT}/Cargo.toml" \
        || die "${REPO_ROOT}/Cargo.toml has no skid-monitor-agent workspace member; pass --repo <path> or --binary <path>"

    command -v cargo >/dev/null 2>&1 || die "cargo not found; install the Rust toolchain (https://rustup.rs) or pass --binary <path>"
    if ! command -v cc >/dev/null 2>&1 && ! command -v gcc >/dev/null 2>&1 && ! command -v clang >/dev/null 2>&1; then
        warn "no C compiler found (cc/gcc/clang); the build may fail on crates that need one."
        warn "Debian/Ubuntu: apt-get install build-essential | Fedora/RHEL: dnf groupinstall 'Development Tools'"
    fi

    log "building skid-monitor-agent (release) from ${REPO_ROOT}"
    (cd "${REPO_ROOT}" && cargo build --release -p skid-monitor-agent)

    local built="${REPO_ROOT}/target/release/skid-monitor-agent"
    [[ -x "${built}" ]] || die "build did not produce ${built}"
    BINARY_PATH="${built}"
}

# ---------------------------------------------------------------------------
# System user / directories
# ---------------------------------------------------------------------------
create_service_user() {
    if id "${SERVICE_USER}" >/dev/null 2>&1; then
        log "service user ${SERVICE_USER} already exists"
    else
        log "creating system user/group ${SERVICE_USER}"
        useradd --system --no-create-home --shell /usr/sbin/nologin --user-group "${SERVICE_USER}"
    fi
    for group in "${EXTRA_GROUPS[@]:-}"; do
        [[ -z "${group}" ]] && continue
        getent group "${group}" >/dev/null 2>&1 || { warn "group ${group} does not exist, skipping"; continue; }
        usermod -aG "${group}" "${SERVICE_USER}"
        log "added ${SERVICE_USER} to group ${group}"
    done
}

create_directories() {
    install -d -m 0755 -o root -g root "${CONFIG_DIR}"
    install -d -m 0750 -o "${SERVICE_USER}" -g "${SERVICE_USER}" "${DATA_DIR}"
}

install_binary() {
    log "installing binary to ${INSTALL_DIR}/${SERVICE_NAME}"
    install -d -m 0755 "${INSTALL_DIR}"
    install -m 0755 -o root -g root "${BINARY_PATH}" "${INSTALL_DIR}/${SERVICE_NAME}"
}

install_config() {
    if [[ -z "${CONFIG_SRC}" ]]; then
        log "no --config given; agent will use legacy env-var configuration (SKID_MONITOR_CLIENT_ADDR)"
        return
    fi
    [[ -f "${CONFIG_SRC}" ]] || die "config file not found: ${CONFIG_SRC}"
    local dest="${CONFIG_DIR}/config.json"
    if [[ -f "${dest}" && "${FORCE}" -ne 1 ]]; then
        log "${dest} already exists, leaving it in place (use --force to overwrite)"
    else
        install -m 0644 -o root -g root "${CONFIG_SRC}" "${dest}"
        log "installed config to ${dest}"
    fi
}

write_env_file() {
    local env_file="${CONFIG_DIR}/agent.env"
    if [[ -f "${env_file}" && "${FORCE}" -ne 1 ]]; then
        log "${env_file} already exists, leaving it in place (use --force to overwrite)"
        return
    fi

    {
        echo "# Managed by install-linux.sh — safe to hand-edit, not overwritten unless --force is used."
        if [[ -n "${CONFIG_SRC}" ]]; then
            echo "SKID_MONITOR_AGENT_CONFIG=${CONFIG_DIR}/config.json"
        else
            echo "SKID_MONITOR_CLIENT_ADDR=${CLIENT_ADDR}"
        fi
        for line in "${EXTRA_ENV_LINES[@]:-}"; do
            [[ -z "${line}" ]] && continue
            [[ "${line}" == *=* ]] || { warn "ignoring malformed --set-env value: ${line}"; continue; }
            echo "${line}"
        done
    } > "${env_file}"

    chown root:root "${env_file}"
    chmod 0600 "${env_file}"
    log "wrote environment file ${env_file}"
}

write_unit() {
    if [[ -f "${UNIT_PATH}" && "${FORCE}" -ne 1 ]]; then
        log "${UNIT_PATH} already exists, leaving it in place (use --force to overwrite)"
        return
    fi

    cat > "${UNIT_PATH}" <<EOF
[Unit]
Description=Skid Monitor host agent
Documentation=https://github.com/LuticaCANARD/skid-monitor
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
User=${SERVICE_USER}
Group=${SERVICE_USER}
EnvironmentFile=${CONFIG_DIR}/agent.env
ExecStart=${INSTALL_DIR}/${SERVICE_NAME}
WorkingDirectory=${DATA_DIR}
Restart=on-failure
RestartSec=5
TimeoutStopSec=30

# Hardening. /proc stays readable (needed for host metrics); only DATA_DIR is writable.
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ProtectHome=true
ReadWritePaths=${DATA_DIR}
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
RestrictRealtime=true
LockPersonality=true
MemoryDenyWriteExecute=true
CapabilityBoundingSet=
AmbientCapabilities=

[Install]
WantedBy=multi-user.target
EOF
    log "wrote unit ${UNIT_PATH}"
}

enable_and_start() {
    systemctl daemon-reload
    systemctl enable "${SERVICE_NAME}"
    if [[ "${START_NOW}" -eq 1 ]]; then
        systemctl restart "${SERVICE_NAME}"
        log "service started; check status with: systemctl status ${SERVICE_NAME}"
    else
        log "service enabled but not started (--no-start given); start with: systemctl start ${SERVICE_NAME}"
    fi
}

# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
require_root
require_linux_systemd

if [[ "${DO_UNINSTALL}" -eq 1 ]]; then
    uninstall
    exit 0
fi

if [[ -z "${BINARY_PATH}" ]]; then
    build_binary
else
    [[ -x "${BINARY_PATH}" ]] || die "--binary ${BINARY_PATH} not found or not executable"
fi

create_service_user
create_directories
install_binary
install_config
write_env_file
write_unit
enable_and_start

cat <<EOF

[install-agent] done.
  binary:      ${INSTALL_DIR}/${SERVICE_NAME}
  config dir:  ${CONFIG_DIR}
  data dir:    ${DATA_DIR}
  service:     systemctl status ${SERVICE_NAME}
  logs:        journalctl -u ${SERVICE_NAME} -f

Note: the default (no --config) setup only sets SKID_MONITOR_CLIENT_ADDR, i.e.
Solo mode pointed at a loopback address. The Solo receiver only accepts numeric
127.0.0.0/8 or ::1 addresses — hostnames and LAN addresses are rejected by design
(see docs/cloud-solo-deployment.md). For a remote/Cloud exporter, pass
--config with an authenticated OTLP config (see
skid-monitor-agent/examples/agent-cloud-config.json) and --set-env to inject
the OAuth client secret.
EOF

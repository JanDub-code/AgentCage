FROM docker.io/library/ubuntu:24.04

ENV DEBIAN_FRONTEND=noninteractive

RUN apt-get update && apt-get install -y --no-install-recommends \
    bash git curl wget ca-certificates tini \
    python3 python3-venv python3-pip \
    nodejs npm ripgrep fd-find \
    && ln -sf /usr/bin/fdfind /usr/local/bin/fd \
    && rm -rf /var/lib/apt/lists/*

ARG UID=10001
ARG GID=10001

ENV AGENT_TOOLS_HOME=/opt/agent-tools

RUN set -eux; \
    mkdir -p "${AGENT_TOOLS_HOME}/bin" "${AGENT_TOOLS_HOME}/lib"; \
    mkdir -p /workspace/project /workspace/session-home; \
    if ! getent group "${GID}" >/dev/null; then \
        groupadd --gid "${GID}" agentcage; \
    fi; \
    existing_user="$(getent passwd "${UID}" | cut -d: -f1 || true)"; \
    if [ -n "${existing_user}" ]; then \
        usermod --gid "${GID}" --home /workspace/session-home --shell /bin/bash "${existing_user}"; \
    else \
        useradd --uid "${UID}" --gid "${GID}" --home-dir /workspace/session-home --no-create-home --shell /bin/bash agentcage; \
    fi; \
    chown -R "${UID}:${GID}" /workspace "${AGENT_TOOLS_HOME}"; \
    chmod 0700 /workspace/session-home

# Agent installer wrapper — checks if agent exists, installs if needed, then exec's
RUN set -eux; \
    printf '%s\n' \
        '#!/bin/sh' \
        'set -eu' \
        'cmd="$1"; shift' \
        'itype="$1"; shift' \
        'target="$1"; shift' \
        'if ! command -v "$cmd" >/dev/null 2>&1; then' \
        '    echo "agentcage: $cmd is not cached yet; installing it into the shared tool cache..." >&2' \
        '    case "$itype" in' \
        '        npm) echo "agentcage: downloading npm package $target..." >&2; npm install --global "$target" >&2 ;;' \
        '        script) echo "agentcage: running installer for $cmd..." >&2; eval "$target" ;;' \
        '    esac' \
        '    if ! command -v "$cmd" >/dev/null 2>&1; then' \
        '        echo "agentcage: error: failed to install $cmd" >&2' \
        '        exit 1' \
        '    fi' \
        '    echo "agentcage: $cmd ready" >&2' \
        'fi' \
        'exec "$@"' \
        > /usr/local/bin/agentcage-ensure; \
    chmod +x /usr/local/bin/agentcage-ensure

ENV HOME=/workspace/session-home \
    XDG_CONFIG_HOME=/workspace/session-home/.config \
    XDG_CACHE_HOME=/workspace/session-home/.cache \
    XDG_DATA_HOME=/workspace/session-home/.local/share \
    XDG_STATE_HOME=/workspace/session-home/.local/state \
    NPM_CONFIG_CACHE=/workspace/session-home/.npm \
    NPM_CONFIG_PREFIX=/opt/agent-tools \
    NPM_CONFIG_UPDATE_NOTIFIER=false \
    NO_UPDATE_NOTIFIER=1 \
    PATH=/opt/agent-tools/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

USER ${UID}:${GID}

WORKDIR /workspace/project
ENTRYPOINT ["/usr/bin/tini", "--"]

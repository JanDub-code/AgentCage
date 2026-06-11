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
    mkdir -p "${AGENT_TOOLS_HOME}"; \
    npm --prefix "${AGENT_TOOLS_HOME}" --cache /tmp/npm-cache install --global \
        @openai/codex \
        @anthropic-ai/claude-code \
        opencode-ai; \
    mkdir -p /tmp/antigravity-home "${AGENT_TOOLS_HOME}/bin"; \
    curl -fsSL https://antigravity.google/cli/install.sh | HOME=/tmp/antigravity-home bash -s -- --dir "${AGENT_TOOLS_HOME}/bin"; \
    test -x "${AGENT_TOOLS_HOME}/bin/codex"; \
    test -x "${AGENT_TOOLS_HOME}/bin/claude"; \
    test -x "${AGENT_TOOLS_HOME}/bin/opencode"; \
    test -x "${AGENT_TOOLS_HOME}/bin/agy"; \
    rm -rf /tmp/npm-cache /tmp/antigravity-home /root/.npm /root/.config

RUN set -eux; \
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
    chown -R "${UID}:${GID}" /workspace; \
    chmod 0700 /workspace/session-home

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

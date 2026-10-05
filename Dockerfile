# Linux amd64 input set. Change pins here, rebuild with ac, and run the release
# smoke checks before shipping. The embedded Dockerfile SHA-256 identifies it.
FROM docker.io/library/ubuntu:24.04@sha256:f610ab94648195aa356059f5b41d6085c9d4d903c072430cdd1af7bdb646106b

ENV DEBIAN_FRONTEND=noninteractive

# The minimal base has no CA bundle. Bootstrap it from a pinned Ubuntu package
# before contacting the HTTPS snapshot; its hash is from Ubuntu's signed index.
ADD https://snapshot.ubuntu.com/ubuntu/20261003T000000Z/pool/main/c/ca-certificates/ca-certificates_20260601~24.04.1_all.deb /tmp/ca-certificates.deb
RUN echo '6bac2a01979e210d9eac1d4d56747ec709ea60654744d66705dc3c36e7629e50  /tmp/ca-certificates.deb' | sha256sum -c - \
    && dpkg-deb --extract /tmp/ca-certificates.deb /tmp/bootstrap-ca \
    && mkdir -p /etc/ssl/certs \
    && cat /tmp/bootstrap-ca/usr/share/ca-certificates/mozilla/*.crt > /etc/ssl/certs/ca-certificates.crt \
    && rm -rf /tmp/ca-certificates.deb /tmp/bootstrap-ca

# Signed Ubuntu package indexes freeze all transitive apt inputs at this date.
# A fixed snapshot remains usable after its normal Valid-Until date.
RUN test "$(dpkg --print-architecture)" = amd64 \
    && printf '%s\n' \
       'Types: deb' \
       'URIs: http://snapshot.ubuntu.com/ubuntu/20261003T000000Z/' \
       'Suites: noble noble-updates noble-security' \
       'Components: main universe' \
       'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg' \
       'Check-Valid-Until: no' > /etc/apt/sources.list.d/ubuntu.sources \
    && apt-get -o APT::Update::Error-Mode=any update \
    && apt-get install -y --no-install-recommends \
       bash git curl wget ca-certificates tini xz-utils \
       python3 python3-venv python3-pip ripgrep fd-find \
    && ln -sf /usr/bin/fdfind /usr/local/bin/fd \
    && mkdir -p /usr/local/share/agentcage \
    && dpkg-query -W > /usr/local/share/agentcage/os-packages \
    && rm -rf /var/lib/apt/lists/*

ARG UID=10001
ARG GID=10001
ENV AGENT_TOOLS_HOME=/opt/agent-tools

# Official Node release (bundles npm 10.9.9), with its published SHA-256.
RUN set -eux; \
    curl -fsSL --retry 3 -o /tmp/node.tar.xz https://nodejs.org/dist/v22.23.3/node-v22.23.3-linux-x64.tar.xz; \
    echo 'df450af89261115ef9f9e3830c3eeb2cc9213b63c720b1af623cb5dcbe2e02de  /tmp/node.tar.xz' | sha256sum -c -; \
    tar -xJf /tmp/node.tar.xz -C /usr/local --strip-components=1; \
    rm /tmp/node.tar.xz; \
    test "$(node --version)" = v22.23.3; \
    test "$(npm --version)" = 10.9.9

# Native agent archives: npm registry dist.integrity for the exact platform
# packages, and Google's official Antigravity release manifest SHA-512.
# Extracting platform archives directly avoids mutable npm install scripts and
# unpinned dependency resolution. Codex keeps its bundled sibling resources.
RUN set -eux; \
    mkdir -p "${AGENT_TOOLS_HOME}/bin" "${AGENT_TOOLS_HOME}/codex" /tmp/agent-archives; \
    cd /tmp/agent-archives; \
    curl -fsSL --retry 3 -o codex.tgz https://registry.npmjs.org/@openai/codex/-/codex-0.160.0-linux-x64.tgz; \
    echo '288ffbdcea86ac7991d7cb3bc9aec4d4da95eab4f4cb7971af4b3c4b5a91da6eb353a41117f1e5479dbd8c02cb879be37549ec453e00868c6dd1ac5be2463df6  codex.tgz' | sha512sum -c -; \
    tar -xzf codex.tgz -C "${AGENT_TOOLS_HOME}/codex" --strip-components=1; \
    ln -s "${AGENT_TOOLS_HOME}/codex/vendor/x86_64-unknown-linux-musl/bin/codex" "${AGENT_TOOLS_HOME}/bin/codex"; \
    curl -fsSL --retry 3 -o claude.tgz https://registry.npmjs.org/@anthropic-ai/claude-code-linux-x64/-/claude-code-linux-x64-2.1.289.tgz; \
    echo '236dd2692604f2f4b015b27e4dcd26465cef26645035a8b20545cafcae51e8156bda53a809d308d92251e0deb1895632eb2c88b77f49362caf3c7a223b29b2cb  claude.tgz' | sha512sum -c -; \
    tar -xzf claude.tgz package/claude; \
    install -m 0755 package/claude "${AGENT_TOOLS_HOME}/bin/claude"; \
    curl -fsSL --retry 3 -o opencode.tgz https://registry.npmjs.org/opencode-linux-x64-baseline/-/opencode-linux-x64-baseline-1.18.34.tgz; \
    echo '4ad582f35b2a006cec8a0eb4d94eb1ba2dedd18fe426d65198977e1f5c584d8ef354b219e08b9c6f55a9d27a3a03bfec0e3e95aad14a74bef5f2eb07f76b0704  opencode.tgz' | sha512sum -c -; \
    tar -xzf opencode.tgz package/bin/opencode; \
    install -m 0755 package/bin/opencode "${AGENT_TOOLS_HOME}/bin/opencode"; \
    curl -fsSL --retry 3 -o agy.tgz https://storage.googleapis.com/antigravity-public/antigravity-cli/1.2.16-5594158052802560/linux-x64/cli_linux_x64.tar.gz; \
    echo 'c31f09265d4f6aef7c03b5105661ae98f9a8365c6c812c68adcc4a5e1f8b8f96c05fd2e4408a52f5e87e1b561ac350f4ffa7ad41a191893be7a3730ccf9d6553  agy.tgz' | sha512sum -c -; \
    tar -xzf agy.tgz antigravity; \
    install -m 0755 antigravity "${AGENT_TOOLS_HOME}/bin/agy"; \
    rm -rf /tmp/agent-archives /root/.npm /root/.config

LABEL org.opencontainers.image.base.name="docker.io/library/ubuntu:24.04" \
      org.opencontainers.image.base.digest="sha256:f610ab94648195aa356059f5b41d6085c9d4d903c072430cdd1af7bdb646106b" \
      io.agentcage.ubuntu-snapshot="20261003T000000Z" \
      io.agentcage.node="22.23.3" \
      io.agentcage.codex="0.160.0" \
      io.agentcage.claude="2.1.289" \
      io.agentcage.opencode="1.18.34" \
      io.agentcage.antigravity="1.2.16"

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
    DISABLE_AUTOUPDATER=1 \
    CODEX_MANAGED_BY_NPM=1 \
    PATH=/opt/agent-tools/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin

USER ${UID}:${GID}
WORKDIR /workspace/project
ENTRYPOINT ["/usr/bin/tini", "-g", "--"]

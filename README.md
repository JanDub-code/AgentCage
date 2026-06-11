# AgentCage

[![Crates.io](https://img.shields.io/badge/crates.io-0.1.0-orange?logo=rust)](https://crates.io/crates/agentcage)
[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Pages](https://img.shields.io/badge/docs-github--pages-blue?logo=github)](https://jandub-code.github.io/AgentCage)

AgentCage (`ac`) runs one AI coding agent inside a disposable **rootless Podman** container.

**Site:** <https://jandub-code.github.io/AgentCage>

It is meant for the times you want to try an experimental, open-source, foreign-hosted, or otherwise high-trust-requiring coding CLI in "YOLO" mode while keeping the agent focused on one repository. The agent can edit the current project, but it does **not** get your host home directory, SSH keys, cloud credentials, browser profile, or the rest of your filesystem.

> AgentCage is a practical containment tool, not a VM and not a perfect sandbox. Read [the security model](#security-model) before using it with secrets or untrusted prompts.

## What it does

- Mounts only the current directory as `/workspace/project`
- Uses a disposable container home by default (`tmpfs`)
- Hides `.agentcage/` from the container so saved environments are not visible through the project mount
- Runs as your host UID/GID in rootless Podman
- Drops Linux capabilities and uses `no-new-privileges`
- Uses a read-only container root filesystem plus explicit writable mounts
- Persists only allowlisted login files for supported agents, unless disabled
- Uses host networking so OAuth localhost callbacks work

## What it does not do

- It does not make the mounted repository safe from edits or deletion
- It does not block network access
- It does not hide credentials that are already inside the repository
- It does not encrypt saved agent login tokens
- It does not provide VM-grade isolation
- It does not implement policy engines, secret scanning, audit logs, dashboards, or diff/apply workflows

## Requirements

- Linux
- Rootless Podman
- Rust/Cargo only when installing from source

Docker is intentionally rejected. Check rootless Podman:

```bash
podman info --format '{{.Host.Security.Rootless}}'
```

The command must print:

```
true
```

## Install

### From a release package

```bash
tar -xzf agentcage-*.tar.gz
cd agentcage-*
./install.sh --no-build
```

By default this installs `ac` to `~/.local/bin`. To choose another location:

```bash
./install.sh --no-build --bin-dir /usr/local/bin
```

### From source

```bash
git clone https://github.com/JanDub-code/AgentCage.git
cd AgentCage
./install.sh
```

This runs `cargo build --release` and installs the resulting binary.

## Quick start

Inside the repository you want the agent to work on:

```bash
ac
```

The first run creates `.agentcage/` and builds the local image:

```
agentcage/agent-tools:local
```

That image build downloads the supported agent tools, so it needs network access.

Run an agent:

```bash
ac codex
ac claude
ac opencode
ac antigravity
```

Open a contained shell:

```bash
ac shell
```

Pass arguments to an agent after `--`:

```bash
ac codex -- --help
```

## Supported agents

| Command | Container binary |
| ------- | ---------------- |
| `ac codex` | `codex` |
| `ac claude` | `claude` |
| `ac opencode` | `opencode` |
| `ac antigravity` | `agy` |

The image installs these tools from their upstream package/install channels during `ac` / `ac init`.

## Security model

| Host path | Container path | Access |
| --------- | -------------- | ------ |
| current directory | `/workspace/project` | read/write |
| `.agentcage/` | hidden by tmpfs at `/workspace/project/.agentcage` | not the host directory |
| container home | `/workspace/session-home` | disposable tmpfs by default |
| host `$HOME` | not mounted | no access |
| saved login staging | `/workspace/login-sync` | selected allowlisted auth files only |

Important boundaries:

- The agent can read and modify anything in the mounted repository
- The agent cannot normally see host `~/.ssh`, `~/.aws`, browser data, or other files outside the repository because host `$HOME` is not mounted
- Network is enabled and uses `--network=host`; this is needed for OAuth callback flows that redirect to `localhost`
- Saved login credentials are copied into the run for the selected agent unless you use `--no-login`
- A malicious agent with valid login credentials can exfiltrate those credentials over the network
- Full persistent environments (`--env`) are convenient but less isolated because the entire container home is reused

For more detail, see [SECURITY.md](SECURITY.md) and [LOGIN_PERSISTENCE.md](LOGIN_PERSISTENCE.md).

## Login persistence

By default, the container home is disposable on every run, but supported agents keep only their allowlisted auth files between runs.

Saved login store:

```
$XDG_DATA_HOME/agentcage/logins/<agent>
# or
~/.local/share/agentcage/logins/<agent>
```

Manage saved logins:

```bash
ac login list
ac login rm codex
ac login rm claude
ac login rm opencode
ac login rm antigravity
```

For untrusted prompts or tests, run without copying saved credentials into the container:

```bash
ac codex --no-login
```

## Full persistent environments

For trusted workflows where you want to keep a whole container home, including settings and caches:

```bash
ac codex --env my-env
ac shell --env my-env
```

Manage them:

```bash
ac env list
ac env rm my-env
```

Do not use `--env` for untrusted runs. A persistent home may contain credentials and other long-lived state.

## Shell completions

```bash
source <(ac completions bash)
source <(ac completions zsh)
ac completions fish | source
```

## Build a distribution package

From the repository root:

```bash
scripts/package.sh
```

The package is written to:

```
dist/agentcage-<version>-<target>.tar.gz
```

## License

MIT. See [LICENSE](LICENSE).

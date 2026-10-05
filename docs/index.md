# AgentCage

**`ac`** runs one coding agent against a writable project in a disposable rootless Podman container.

Your host home is not separately mounted. Network access is enabled, and normal runs reuse selected saved login files. The agent can read, edit, and delete project files; project edits, the image, and saved logins survive container removal. Use `--no-login` to skip saved-login sync. Rootless containers share the host kernel.

- [GitHub repository](https://github.com/JanDub-code/AgentCage)
- [README and requirements](https://github.com/JanDub-code/AgentCage/blob/main/README.md#requirements)
- [Security model](https://github.com/JanDub-code/AgentCage/blob/main/SECURITY.md)
- [Login persistence](https://github.com/JanDub-code/AgentCage/blob/main/LOGIN_PERSISTENCE.md)

## Install

Runtime: Linux x86_64, rootless Podman 4.9.3+, crun 1.14.1+, systemd cgroups v2 with CPU, memory, and PID delegation. Docker is not supported. Read the README's validation limits before treating a host configuration as tested.

Source installation requires Git and Rust/Cargo:

```bash
git clone https://github.com/JanDub-code/AgentCage.git AgentCage &&
cd AgentCage &&
./install.sh &&
export PATH="$HOME/.local/bin:$PATH" &&
hash -r &&
command -v ac &&
ac --help
```

For binary packages and other installation methods, follow the [installation instructions](https://github.com/JanDub-code/AgentCage/blob/main/README.md#install).

## Quick start

Choose the project the agent should edit, not the installer checkout or your home directory. Replace the example path:

```bash
cd /path/to/project &&
ac &&
ac codex
```

The first `ac` initializes project state and builds the local image with network downloads. Agent commands require that image; `ac init` is an explicit alias. For a run without selected saved credentials, after initialization:

```bash
ac codex --no-login
```

Credentials already in the project remain visible. `--env NAME` reuses the entire container home and cannot be combined with `--no-login`. Agent options follow `--`, for example `ac codex -- --help`.

## Agent commands

| Command | Agent |
| --- | --- |
| `ac codex` | OpenAI Codex CLI |
| `ac claude` | Anthropic Claude Code |
| `ac opencode` | OpenCode |
| `ac antigravity` | Antigravity CLI (`agy`) |

For a disposable contained shell, run `ac shell`. Tool versions are specified by the image recipe embedded in the installed binary.

Pinned OpenCode 1.18.34 syncs account credentials as `account.json` and provider credentials as `auth.json`. The default login sync does not copy its SQLite database, chats, logs, or history to the host store. See [OpenCode account compatibility and validation limits](https://github.com/JanDub-code/AgentCage/blob/main/LOGIN_PERSISTENCE.md#opencode-11834-account-compatibility).

## Recovery and boundaries

If the image is missing or outdated, run `ac` again in the project. Podman preflight errors describe unmet runtime requirements. Follow the [tutorial troubleshooting steps](https://github.com/JanDub-code/AgentCage/blob/main/TUTORIAL.md#troubleshooting).

The project is writable, host networking includes host services, and available tokens can be sent over the network. Default homes are disposable; allowlisted auth and full `--env` homes are persistent state. Container removal does not remove that state or undo project edits. See the security model and login guide above.

## License

MIT.

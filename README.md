# AgentCage

[![MIT License](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![GitHub Pages](https://img.shields.io/badge/docs-github--pages-blue?logo=github)](https://jandub-code.github.io/AgentCage)

AgentCage (`ac`) runs one coding agent against the current directory in a disposable **rootless Podman** container.

The project is mounted **read/write**. Your host home is not separately mounted. Network access is enabled, and normal agent runs reuse selected saved login files. Project edits, the built image, and saved logins survive container removal. Use `--no-login` to omit saved credentials from a run.

**Site:** [AgentCage](https://jandub-code.github.io/AgentCage) · [Step-by-step tutorial](TUTORIAL.md)

Rootless containers share the host kernel. Read [the security model](#security-model) before using untrusted agents or prompts. Choose a project directory, not your home directory: everything inside the chosen directory is exposed to the agent except the overlaid `.agentcage/` state directory.

## What it does

- Mounts the current project as `/workspace/project`
- Uses a disposable container home by default
- Overlays project-local `.agentcage/` with an empty tmpfs
- Runs through rootless Podman as your host UID/GID
- Drops capabilities, uses `no-new-privileges`, and keeps the image root read-only
- Enforces CPU, memory, and PID limits on the required cgroups configuration
- Persists allowlisted login files for the selected agent unless disabled
- Uses host networking, including access to host localhost services

The repository remains writable, network access is unrestricted, and saved login files are not encrypted. This tool does not provide VM isolation, policy engines, secret scanning, audit logs, dashboards, or diff/apply workflows.

## Requirements

The supported runtime baseline is deliberately narrow:

| Component | Required configuration |
| --- | --- |
| Host | Linux x86_64 |
| Podman | 4.9.3 or newer, local rootless mode |
| OCI runtime | crun 1.14.1 or newer |
| cgroups | v2, systemd manager, delegated `cpu`, `memory`, and `pids` controllers |
| Source installation | Git and Rust/Cargo 1.85 or newer, as declared in [Cargo.toml](https://github.com/JanDub-code/AgentCage/blob/main/Cargo.toml) |
| curl installation | Source requirements above, plus curl |
| Binary package | POSIX shell, tar, and SHA-256 verification tools; Rust and Git are not needed |

Docker, remote Podman, rootful operation, runc, and cgroups v1 are rejected. The preflight checks the required configuration before starting an agent.

Inspect your runtime:

```bash
podman version
podman info --format '{{.Host.Security.Rootless}}'
podman info --format '{{.Host.OCIRuntime.Name}} {{.Host.OCIRuntime.Version}}'
podman info --format '{{.Host.CgroupsVersion}} {{.Host.CgroupManager}} {{.Host.CgroupControllers}}'
```

The rootless check must print `true`; the runtime must be crun, with `v2`, `systemd`, and the three required controllers. If preflight fails, follow its diagnostic and configure the user Podman runtime before retrying.

The runtime validation target is Ubuntu 24.04 / Linux x86_64 with Podman 4.9.3, crun 1.14.1, systemd, and cgroups v2. Newer versions still need the same containment smoke checks. Native Windows/macOS, ARM, SELinux hosts, and upstream OAuth/refresh flows are not covered by that target and must not be inferred from a successful Cargo build. See [runtime validation](#runtime-validation).

## Install

### Quick installation via curl

This method downloads the installer, clones this repository's current `main`, builds with the lockfile, and installs the binary. It requires Git and Rust/Cargo. The installer runs only after a successful, nonempty download.

```bash
install_script=$(curl -fsSL https://raw.githubusercontent.com/JanDub-code/AgentCage/main/install.sh) &&
{ [ -n "$install_script" ] || { printf '%s\n' 'Installer download was empty.' >&2; false; }; } &&
printf '%s\n' "$install_script" | sh
```

To choose a directory with the bootstrap installer:

```bash
install_script=$(curl -fsSL https://raw.githubusercontent.com/JanDub-code/AgentCage/main/install.sh) &&
{ [ -n "$install_script" ] || { printf '%s\n' 'Installer download was empty.' >&2; false; }; } &&
printf '%s\n' "$install_script" | sh -s -- --bin-dir "$HOME/.local/bin"
```

`--no-build` requires an existing checkout build or binary package; it is rejected for bootstrap installation.

### From a binary package

Use a matching archive and checksum from the same trusted [release](https://github.com/JanDub-code/AgentCage/releases) or from `scripts/package.sh`. The example below is for version 0.1.0, Linux x86_64. Run it in the download directory containing these exact filenames:

```bash
sha256sum -c agentcage-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256 &&
tar -xzf agentcage-0.1.0-x86_64-unknown-linux-gnu.tar.gz &&
cd agentcage-0.1.0-x86_64-unknown-linux-gnu &&
./install.sh --no-build
```

The exact directory avoids ambiguity when an archive and extracted directory coexist or several versions have been downloaded. Replace both filenames and the directory together when selecting a different release.

The package's `RELEASE` file identifies its version, target, source revision/state, binary checksum, and image recipe checksum. The installer also verifies the packaged binary against that metadata. A checksum detects corruption; it does not authenticate the publisher. Obtain the archive and checksum through a trusted release source. Release signatures are not currently provided.

By default the script installs to `~/.local/bin`. To choose another writable directory:

```bash
./install.sh --no-build --bin-dir "$HOME/bin"
```

### From source

```bash
git clone https://github.com/JanDub-code/AgentCage.git AgentCage &&
cd AgentCage &&
./install.sh
```

This builds a locked release for `x86_64-unknown-linux-gnu`. `CARGO_TARGET_DIR` is honored; otherwise the script uses the checkout's `target/`. The script explicitly selects the target and output directory, overriding Cargo configuration for those two settings. Cross-target packaging is rejected.

Cargo can also install the repository directly:

```bash
cargo install --git https://github.com/JanDub-code/AgentCage.git --locked
```

Cargo installs to `${CARGO_HOME:-$HOME/.cargo}/bin`. A source installation follows the chosen Git revision; checkout a reviewed tag or commit first when fixed source inputs are required.

### Verify PATH

For either installation method in the current Bash/Zsh shell:

```bash
export PATH="$HOME/.local/bin:${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" &&
hash -r &&
command -v ac &&
ac --help
```

Add that PATH setting to your shell's startup file if you want it to persist. It includes a custom `CARGO_HOME` when set. For a custom script `--bin-dir`, also add that chosen directory.

## Quick start

Inside the project you want the agent to work on:

```bash
cd /path/to/project &&
ac &&
ac codex
```

Replace `/path/to/project` with the actual project directory. The first `ac` creates `.agentcage/` and builds `agentcage/agent-tools:local` from the image recipe embedded in the binary. The build downloads tools and needs network access. `ac init` is an explicit alias. An agent command requires that image to be ready; if it is missing or the embedded recipe changed, run `ac` first.

Choose one agent:

```bash
ac codex
ac claude
ac opencode
ac antigravity
```

For a disposable shell with no login sync:

```bash
ac shell
```

AgentCage options come before `--`; everything after it is passed to the agent:

```bash
ac codex -- --help
ac codex --no-login -- exec -
```

`ac codex --help` shows wrapper help. `ac codex -- --help` shows the agent's help. `--env NAME` and `--no-login` cannot be combined.

Foreground stdin is forwarded, including pipes; a TTY is allocated only when stdin and stdout are terminals. The session banner goes to stderr so agent stdout can be captured unchanged. The child's exit status is preserved, including 130 for interruption. A wrapper or login-finalization failure returns a nonzero result; a failed finalize does not mask an already nonzero child result.

## Agent commands and image updates

| Command | Container binary |
| --- | --- |
| `ac codex` | `codex` |
| `ac claude` | `claude` |
| `ac opencode` | `opencode` |
| `ac antigravity` | `agy` |

The image pins its base digest, package snapshot, and agent artifacts in [Dockerfile](Dockerfile). These fixed inputs improve predictable upgrades; they are not a promise of bit-for-bit reproducible image bytes.

To update tools, review the version and checksum changes, rebuild the `ac` binary, and run `ac` in your project. The embedded recipe fingerprint detects a changed recipe. Startup/version smoke checks must be repeated for every declared tool before release.

## Security model

| Host path/state | Container path | Access |
| --- | --- | --- |
| current directory | `/workspace/project` | read/write |
| `.agentcage/` | empty tmpfs over `/workspace/project/.agentcage` | host state hidden through this project path |
| container home | `/workspace/session-home` | disposable tmpfs by default |
| host `$HOME` | no separate mount | anything under the chosen project is still exposed |
| selected login staging | `/workspace/login-sync` | allowlisted files for the selected agent |
| `--env NAME` home | `/workspace/session-home` | full persistent home, read/write |

- The agent can read, modify, and delete files in the mounted project. Keep backups or use a throwaway clone.
- Host networking enables both internet access and access to services on the host.
- Saved login credentials are available to the selected agent unless you use `--no-login`; they can be sent over the network.
- Credentials already inside the project remain visible with `--no-login`. That option prevents saved-login sync; it does not block authentication performed inside the run.
- Removing the container discards its disposable home, while project edits, the image, and saved state remain.
- `--env` reuses the entire container home, including caches, settings, and any credentials.

See [SECURITY.md](SECURITY.md) and [LOGIN_PERSISTENCE.md](LOGIN_PERSISTENCE.md).

## Login persistence

Normal disposable-home agent runs persist only allowlisted auth files:

```text
$XDG_DATA_HOME/agentcage/logins/<agent>
# or, when XDG_DATA_HOME is unset:
~/.local/share/agentcage/logins/<agent>
```

```bash
ac login list
ac login rm codex
ac codex --no-login
```

`login list` reports recognizable locally saved files, not proof that a provider accepts a token. `login rm` removes the selected local store and prevents older open sessions from restoring it. It does not revoke provider tokens or remove credentials from persistent `--env` homes.

Concurrent sessions take snapshots. An unchanged session does not overwrite a newer login; incompatible concurrent changes produce a persistence error instead of silently replacing credentials. See [login recovery](LOGIN_PERSISTENCE.md#concurrent-sessions-and-local-removal).

For pinned OpenCode 1.18.34, provider credentials still use `auth.json`. Account credentials and the selected account/organization are restored from and exported to the allowlisted `account.json`. An adapter runs inside the disposable home; its SQLite database, chats, logs, and history are not copied into the host login store. See [OpenCode account compatibility](LOGIN_PERSISTENCE.md#opencode-11834-account-compatibility) for commands and validation limits. A full `--env` home instead retains all its state.

## Full persistent environments

For a trusted workflow:

```bash
ac codex --env my-env
ac shell --env my-env
ac env list
ac env rm my-env
```

Homes live under `.agentcage/envs/NAME/home` in the current project and remain until removed. `env rm` deletes that home and its contents; finish sessions using it first. Avoid `--env` for untrusted runs.

## Cleanup

```bash
ac clean
```

Cleanup is global for the current rootless Podman user. It removes only stopped containers with AgentCage's ownership label `io.agentcage.managed=true`, by ID, without forcing termination. Running containers and unlabelled older containers are left alone. A listing or removal failure returns a nonzero result.

## Shell completions

For the current shell:

```bash
source <(ac completions bash)
```

```zsh
autoload -Uz compinit
compinit
source <(ac completions zsh)
```

```fish
ac completions fish | source
```

Wrapper completions stop after `--`; the agent owns arguments after that delimiter.

## Build and validate a distribution package

```bash
cargo test --locked
scripts/package.sh
```

The script produces `dist/agentcage-<version>-x86_64-unknown-linux-gnu.tar.gz` and its `.sha256` file. Build inputs and release target are recorded in `RELEASE`.

## Runtime validation

Fast tests exercise parser, process, persistence, cleanup, and installer contracts with disposable fixtures. They do not prove kernel isolation or upstream login compatibility.

From a source checkout of the revision being tested, run the opt-in live containment and agent-start checks on the required runtime with [scripts/smoke-podman.py](https://github.com/JanDub-code/AgentCage/blob/main/scripts/smoke-podman.py):

```bash
python3 scripts/smoke-podman.py --binary /absolute/path/to/ac --run --required
```

Replace the binary path with the tested build. The checks use their own labelled disposable fixtures and no user login store. Without `--run` they report an explicit unverified result (exit 77). `--required` makes an unavailable runtime a failure. A missing runtime is missing evidence, not a successful live test.

Upstream OAuth and token refresh need dedicated test accounts and provider/browser interaction. They remain separate from help/version startup checks. Further architectures and SELinux configurations need their own live runs before being advertised as validated.

## License

MIT. See [LICENSE](LICENSE).

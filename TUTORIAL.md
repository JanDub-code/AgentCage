# AgentCage tutorial

This guide installs `ac`, initializes the intended project, and starts one agent. The agent can write the project and use host networking; normal runs reuse selected saved login credentials. Use `--no-login` to skip that sync.

## 1. Check requirements

You need Linux x86_64, rootless Podman 4.9.3 or newer, crun 1.14.1 or newer, and systemd cgroups v2 with delegated CPU, memory, and PID controllers. Source installation also needs Git and Rust/Cargo; curl installation adds curl. See the [requirements and validation target](README.md#requirements).

```bash
podman version
podman info --format '{{.Host.Security.Rootless}}'
podman info --format '{{.Host.OCIRuntime.Name}} {{.Host.OCIRuntime.Version}}'
podman info --format '{{.Host.CgroupsVersion}} {{.Host.CgroupManager}} {{.Host.CgroupControllers}}'
```

Rootless must print `true`; the other results must describe crun, `v2`, `systemd`, and `cpu`, `memory`, `pids`. Docker and remote/rootful Podman are not supported. Runtime preflight gives an error before agent startup when these assumptions are missing.

## 2. Install

Choose one path.

### From a binary package

Download a matching archive and checksum from the same trusted [release](https://github.com/JanDub-code/AgentCage/releases), or use a package built with `scripts/package.sh`. This exact example uses version 0.1.0:

```bash
sha256sum -c agentcage-0.1.0-x86_64-unknown-linux-gnu.tar.gz.sha256 &&
tar -xzf agentcage-0.1.0-x86_64-unknown-linux-gnu.tar.gz &&
cd agentcage-0.1.0-x86_64-unknown-linux-gnu &&
./install.sh --no-build
```

Replace archive, checksum, and directory names together for another version. The exact directory works even with several archives in the download directory. Checksum verification detects corruption; it relies on a trusted source for publisher identity. Git and Rust are not needed for the binary package.

### From source

```bash
git clone https://github.com/JanDub-code/AgentCage.git AgentCage &&
cd AgentCage &&
./install.sh
```

This installs a locked release build from the cloned revision. The install directory is `~/.local/bin`. To choose another writable directory:

```bash
./install.sh --bin-dir "$HOME/bin"
```

See the [curl and Cargo Git alternatives](README.md#install). Bootstrap installation builds source and does not accept `--no-build`.

## 3. Verify PATH

In the current Bash or Zsh shell, for either installation method:

```bash
export PATH="$HOME/.local/bin:${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" &&
hash -r &&
command -v ac &&
ac --help
```

For Fish, use `fish_add_path $HOME/.local/bin` for a script install, or `fish_add_path $CARGO_HOME/bin` for Cargo when `CARGO_HOME` is set (`fish_add_path $HOME/.cargo/bin` otherwise). For a custom script `--bin-dir`, add that chosen directory. To keep the Bash/Zsh setting across sessions, add it once to your shell's startup file.

## 4. Initialize the right project and start an agent

Leave the AgentCage installer checkout and enter the project the agent should edit. Replace the example path with your actual project:

```bash
cd /path/to/project &&
ac &&
ac codex
```

Do not choose your host home as the project: everything inside the chosen directory is exposed. The first run creates `.agentcage/` and builds `agentcage/agent-tools:local` from the embedded recipe. It needs network access and may take several minutes. `ac init` is the explicit equivalent.

## 5. Run another session

```bash
ac codex
```

Other choices are `ac claude`, `ac opencode`, and `ac antigravity`. For a disposable contained shell with no login sync, run `ac shell`.

Wrapper options come before `--`; agent options follow it:

```bash
ac codex -- --help
ac codex --no-login -- exec -
```

`ac codex --help` shows AgentCage help; `ac codex -- --help` shows agent help. Stdin pipes are forwarded. Agent stdout stays separate from the wrapper's stderr banner, and the agent's exit code is preserved, including interruption code 130.

## 6. Run without saved credentials

```bash
ac codex --no-login
```

This skips saved-login sync. Credentials already in the project remain accessible, and network access and project writes remain enabled. Container removal does not undo project changes.

## 7. Manage saved logins

```bash
ac login list
ac login rm codex
```

Replace `codex` with another supported agent to remove its store. Local removal prevents older sessions from restoring the store, but it does not revoke the token at its provider, erase credentials already copied into a running session, or clear persistent environment homes.

Overlapping sessions with incompatible credential changes report a persistence conflict. Finish them and retry authentication in one fresh session. See [login persistence and recovery](LOGIN_PERSISTENCE.md#concurrent-sessions-and-local-removal).

For pinned OpenCode 1.18.34 account operations:

```bash
ac opencode -- console login
ac opencode -- console logout you@example.com
```

Replace the email with the account to remove. Default sync retains account credentials as `account.json` and provider credentials as `auth.json`; it does not save the OpenCode SQLite database or its chats/history. Real provider login and refresh are separate from synthetic adapter/logout checks and remain unverified. See [OpenCode compatibility](LOGIN_PERSISTENCE.md#opencode-11834-account-compatibility).

## 8. Optional persistent environments

For trusted workflows that need the entire container home:

```bash
ac codex --env my-env
ac shell --env my-env
ac env list
ac env rm my-env
```

Homes are project-local under `.agentcage/envs/NAME/home` and can contain credentials, settings, and caches. `--env` cannot be combined with `--no-login`. Finish sessions before removing their environment; removal deletes its home.

## 9. Shell completions

Run the matching setup in that shell:

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

Wrapper suggestions stop after `--`.

## Troubleshooting

- **`ac` not found:** check `command -v ac` and the PATH step above.
- **Unexpected help/options:** in Bash use `type -a ac` to locate older installations. Reinstall the desired build, then `hash -r`.
- **Image missing or outdated:** run `ac` in the target project. The embedded recipe comes from the installed binary, so rebuilding the checkout alone does not update a different installed `ac`.
- **Podman preflight failure:** inspect the printed Podman cause and requirements above. Configure rootless crun, systemd cgroups v2, and controller delegation; protections are not silently lowered.
- **Agent option rejected:** put agent options after `--`, for example `ac codex -- --help`.
- **Persistence conflict/local removal during a run:** finish overlapping sessions and start a fresh login-enabled session if needed. Use `--no-login` when saved auth is unwanted.
- **Stopped container left behind:** `ac clean` removes only labelled stopped AgentCage containers for the current Podman user. It leaves running and unlabelled containers alone.

To build and install the current checkout again:

```bash
./install.sh &&
export PATH="$HOME/.local/bin:${CARGO_HOME:-$HOME/.cargo}/bin:$PATH" &&
hash -r &&
command -v ac &&
ac --help
```

Read [SECURITY.md](SECURITY.md) before using untrusted agents. OAuth/refresh and additional host configurations need live checks; local file detection or a help/version launch is not that evidence.

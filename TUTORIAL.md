# AgentCage tutorial

This guide installs `ac` and runs an agent in a contained project workspace.

## 1. Requirements

You need:

- Linux
- rootless Podman
- Rust/Cargo only when building from source

Check rootless Podman:

```bash
podman info --format '{{.Host.Security.Rootless}}'
```

Expected output:

```text
true
```

Docker is not supported.

## 2. Install

### From a release package

```bash
tar -xzf agentcage-*.tar.gz
cd agentcage-*
./install.sh --no-build
```

### From source

```bash
git clone <repo-url>
cd agentcage
./install.sh
```

Default install path:

```text
~/.local/bin/ac
```

Custom install path:

```bash
./install.sh --bin-dir /usr/local/bin
```

## 3. Ensure `ac` is in PATH

For bash:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
hash -r
```

For zsh:

```bash
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc
source ~/.zshrc
hash -r
```

Verify:

```bash
which ac
ac --help
```

## 4. Initialize a project

Go to the repository you want the agent to work on:

```bash
cd ~/src/my-project
ac
```

The first run builds the local container image. This can take a while and needs network access.

## 5. Run agents

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

Pass raw arguments to an agent after `--`:

```bash
ac codex -- --help
```

## 6. Use safer no-login runs

For untrusted prompts or tests, avoid copying saved agent credentials into the run:

```bash
ac codex --no-login
```

The agent can still edit the mounted repository, but it will not receive saved login files.

## 7. Manage logins

Normal runs persist only allowlisted auth files for supported agents:

```bash
ac login list
ac login rm codex
ac login rm claude
ac login rm opencode
ac login rm antigravity
```

See [LOGIN_PERSISTENCE.md](./LOGIN_PERSISTENCE.md).

## 8. Optional persistent environments

For trusted workflows where you want to keep the entire container home:

```bash
ac codex --env my-env
ac shell --env my-env
```

Manage environments:

```bash
ac env list
ac env rm my-env
```

Do not use persistent environments for untrusted runs.

## 9. Shell completions

```bash
source <(ac completions bash)
source <(ac completions zsh)
ac completions fish | source
```

## 10. Troubleshooting old binaries

If `ac --help` does not show the expected commands/options, your shell may be using an older binary.

```bash
cargo build --release
./install.sh --no-build
hash -r
which -a ac
```

Install to the directory that appears first in `which -a ac`.

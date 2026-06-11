# Security model

AgentCage is designed for one narrow job: run a single AI coding agent against one repository without mounting your host home directory.

It is a containment layer, not a VM-grade sandbox.

## Protected by design

- Host `$HOME` is not mounted.
- Common host secrets such as `~/.ssh`, `~/.aws`, browser profiles, and shell config are not visible unless they are inside the project you mounted.
- Only the current directory is mounted as `/workspace/project`.
- `.agentcage/` is hidden inside the container by a tmpfs mounted over `/workspace/project/.agentcage`.
- Container home is disposable tmpfs unless `--env NAME` is used.
- The container runs rootless through Podman, as the current UID/GID.
- The runtime uses `--cap-drop=ALL`, `--security-opt=no-new-privileges`, a read-only image root, PID/memory/CPU limits, and explicit writable mounts.

## Not protected

- The mounted repository is writable. A rogue agent can edit, delete, or overwrite files in the repo.
- Network is enabled and uses host networking. Agents can reach the internet.
- Secrets committed to the repository are visible to the agent.
- Saved agent login tokens are available to that agent during login-enabled runs.
- Full persistent homes created with `--env NAME` may contain long-lived credentials and state.
- Rootless containers reduce risk but are not equivalent to virtual machines.

## Recommended usage

For normal agent use:

```bash
ac codex
ac opencode
```

For untrusted prompts, demos, or tests, do not copy saved credentials into the run:

```bash
ac codex --no-login
```

For the safest workflow, use a disposable throwaway clone of your repository and separate low-privilege accounts/tokens for agent logins.

Avoid `--env NAME` unless you trust the workflow. It persists the whole container home.

## Login credentials

AgentCage persists only allowlisted auth files for supported agents. They are stored under:

```text
$XDG_DATA_HOME/agentcage/logins/<agent>
# or
~/.local/share/agentcage/logins/<agent>
```

Files and directories are set to private permissions where possible. Symlinked and non-regular credential files are rejected. Oversized login files are rejected.

See [LOGIN_PERSISTENCE.md](./LOGIN_PERSISTENCE.md) for details.

## Reporting security issues

If this repository is hosted on a platform with private security advisories, use that. Otherwise, open an issue with a minimal reproduction and avoid posting real secrets or tokens.

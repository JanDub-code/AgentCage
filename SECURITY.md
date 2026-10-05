# Security model

AgentCage runs one coding agent against one chosen directory without a separate host-home mount. It uses rootless Podman and shares the host kernel. It is a containment layer, not VM isolation.

The selected project is writable and networking is enabled. Normal agent runs receive selected saved login files. Container removal preserves project changes, the image, and saved state.

## Filesystem boundary

- The current directory is mounted at `/workspace/project` as read/write.
- Host `$HOME` is not separately mounted. If you select your home directory or a directory containing secrets as the project, those files are exposed.
- Project-local `.agentcage/` is overlaid with an empty tmpfs using `notmpcopyup`, so saved environments cannot be read through that project path.
- The container home, `/workspace/session-home`, is disposable tmpfs by default.
- `--env NAME` instead mounts a full persistent home from the current project's `.agentcage/envs/NAME/home`.
- Login-enabled disposable runs mount a private staging directory at `/workspace/login-sync` containing allowlisted credentials for the selected agent.

The container root filesystem is read-only, with explicit writable mounts for the project, home, and runtime temporary state. Linux capabilities are dropped and `no-new-privileges` is set. CPU, memory, and PID limits require the runtime configuration in [README.md](README.md#requirements).

## Accessible data and network

The agent can edit or delete project files and read any project secrets. Host networking allows access to the internet and services on the host, including localhost. It is used for login callback flows; it also permits sending project contents or available credentials over the network.

`--no-login` prevents copying from and writing to the saved-login store for that run. It does not hide credentials already in the project, block authentication performed in the run, or disable networking. It cannot be combined with `--env`, whose persistent home may already contain credentials.

The default home disappears after the container exits. This does not undo project edits or delete the built image or login store. A forcibly killed wrapper or host interruption can leave temporary login staging until a later cleanup.

## Login credentials

The per-user store is:

```text
$XDG_DATA_HOME/agentcage/logins/<agent>
# or, when XDG_DATA_HOME is unset:
~/.local/share/agentcage/logins/<agent>
```

Saved tokens are not encrypted. Files and directories use private permissions. Only allowlisted regular credential files within the staging root are accepted; symlinked path components, non-regular files, and files over 10 MiB are rejected.

Persistence uses per-agent transactions. Unchanged snapshots cannot replace a newer store, divergent changes fail visibly, and local removal prevents older sessions from restoring the removed login. `ac login rm AGENT` removes local copies only. It does not revoke tokens at the provider, erase copies already handed to a running agent, or clear persistent environment homes.

See [LOGIN_PERSISTENCE.md](LOGIN_PERSISTENCE.md) for file paths and recovery.

For pinned OpenCode 1.18.34, normal login sync converts account credentials and the selected account/organization between its disposable-home SQLite tables and the allowlisted `account.json`. It does not copy the database, chat sessions, logs, or history to the host login store. Provider `auth.json` sync is unchanged. A full `--env` home still persists everything it contains. Adapter tests do not establish provider OAuth or refresh compatibility.

## Recommended usage

Use a throwaway clone with no project secrets for untrusted experiments:

```bash
cd /path/to/throwaway-project &&
ac &&
ac codex --no-login
```

When login is needed, use separate accounts or tokens with permissions appropriate to the workflow. Avoid persistent `--env` homes for untrusted runs.

## Validation limits

A constructed Podman command and passing mock tests do not prove mount semantics, resource enforcement, or upstream OAuth behavior. The required Linux x86_64 / rootless crun / systemd cgroups v2 baseline and live checks are described in [runtime validation](README.md#runtime-validation). SELinux and other architectures need separate validation. Help/version startup does not establish token validity or refresh compatibility.

## Reporting security issues

Use the repository host's private security advisory channel when available. Otherwise provide a minimal issue reproduction without real credentials, tokens, or personal files.

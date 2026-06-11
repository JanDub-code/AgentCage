# Login persistence and safety

AgentCage keeps the container home disposable by default, but it can persist selected login files for supported agents so you do not need to authenticate on every run.

## How it works

- `$PWD` is mounted as `/workspace/project`.
- Host `$HOME` is not mounted.
- Container home is `/workspace/session-home`.
- Without `--env`, container home is a disposable tmpfs.
- `.agentcage/` is hidden from the agent by a tmpfs mounted over `/workspace/project/.agentcage`.
- For supported agents, only allowlisted auth files are persisted.

The host login store is:

```text
$XDG_DATA_HOME/agentcage/logins/<agent>
# or
~/.local/share/agentcage/logins/<agent>
```

On each login-enabled run AgentCage:

1. creates a private temporary staging directory,
2. copies saved allowlisted auth files into it,
3. bind-mounts that staging directory into the container as `/workspace/login-sync`,
4. copies those files into `$HOME` inside the container before the agent starts,
5. copies only the same allowlisted files back after the agent exits,
6. deletes the staging directory.

Stale staging directories for the same agent are cleaned on later runs if they are owned by the current user, not owned by a live process, and older than 24 hours.

## Persisted files

```text
codex:       auth.json, .codex/auth.json
claude:      .claude/.credentials.json, .claude.json
opencode:    .local/share/opencode/auth.json, .local/share/opencode/account.json
antigravity: .gemini/antigravity-cli/antigravity-oauth-token
```

AgentCage sets private permissions where possible:

- directories: `0700`
- files: `0600`

Symlinked credentials, directories, non-regular files, and oversized login files are rejected.

## Disable login sync for a run

For untrusted prompts or tests:

```bash
ac codex --no-login
```

`--no-login` cannot be combined with `--env`, because `--env` reuses a full persistent home that may already contain credentials.

## Full persistent homes

`--env NAME` keeps the whole container home:

```bash
ac codex --env my-env
ac shell --env my-env
```

This is useful for trusted workflows that need settings, caches, or custom setup. It is less isolated than the default mode because all home state persists.

Manage environments:

```bash
ac env list
ac env rm my-env
```

## Network and OAuth

AgentCage uses host networking. This is intentional: many CLI login flows start a localhost callback server and redirect the browser to URLs such as:

```text
http://localhost:1455/auth/callback
```

AgentCage does not provide an offline mode. If you need egress blocking, enforce it outside AgentCage with your OS firewall, network namespace setup, or a proxy.

## Limitations

Saved tokens are not encrypted. During a login-enabled run, the selected agent can read its token files and can send them over the network. Use separate low-privilege accounts/tokens for risky workflows and remove saved logins when done:

```bash
ac login list
ac login rm codex
ac login rm claude
ac login rm opencode
ac login rm antigravity
```

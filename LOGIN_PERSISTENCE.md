# Login persistence and safety

By default, AgentCage uses a disposable container home and persists selected auth files for the chosen agent. Normal runs receive those saved files. They remain readable by the agent and can be sent over the enabled network.

Use `ac codex --no-login` to skip saved-login sync for a run. Project credentials remain accessible, and the agent can still authenticate during that run.

## How it works

- The current project is mounted read/write as `/workspace/project`.
- Host `$HOME` is not separately mounted; do not choose it as your project.
- Container home is `/workspace/session-home`, disposable tmpfs unless `--env` is used.
- An empty tmpfs overlays the project's `.agentcage/` state directory.
- Only the selected agent's allowlisted auth files are synced.

The per-user host store is:

```text
$XDG_DATA_HOME/agentcage/logins/<agent>
# or, when XDG_DATA_HOME is unset:
~/.local/share/agentcage/logins/<agent>
```

A login-enabled run:

1. Takes a snapshot of the selected agent's store under a short per-agent transaction.
2. Creates a private temporary staging directory and copies allowlisted saved files into it.
3. Mounts staging as `/workspace/login-sync` and copies the files into the disposable container home before starting the agent.
4. Copies the same allowlisted paths back after the agent exits.
5. Validates the returned files, checks for concurrent changes or local removal, then publishes the complete accepted credential snapshot atomically.
6. Removes its staging directory. If saving a changed, validated snapshot fails, including a concurrent-store conflict, it keeps a private recovery copy and prints its path, including when the agent already exited with an error. Incomplete or invalid exports and sessions invalidated by local `login rm` are discarded.

An abrupt wrapper termination can leave staging behind. A later login-enabled run cleans same-agent staging directories only when they belong to the current user, are not owned by a live process, and are older than 24 hours.

A copy explicitly retained after a save failure is excluded from automatic cleanup when its recovery marker was saved successfully. If that marker also fails, the diagnostic warns that normal 24-hour cleanup applies. Keep the printed path private: it contains credentials. Resolve the reported filesystem problem, preserve any needed returned credentials before authenticating again, and remove the recovery directory when it is no longer needed. It is never imported automatically or allowed to restore a locally removed login.

## Persisted files

```text
codex:       auth.json, .codex/auth.json
claude:      .claude/.credentials.json, .claude.json
opencode:    .local/share/opencode/auth.json, .local/share/opencode/account.json
antigravity: .gemini/antigravity-cli/antigravity-oauth-token
```

Directories use `0700`; files use `0600`. Credential imports accept only regular files inside the staging root, reject symlinked path components and non-regular files, and limit each file to 10 MiB.

Codex JSON and Claude API-key/OAuth fields are checked against the pinned formats before saving. Invalid JSON, wrong field types, and incomplete token structures fail visibly and keep the accepted store. Format checks do not verify a token's signature or provider acceptance. Keep unrelated files outside the managed login store; unexpected entries prevent replacement rather than being deleted.

The allowlist depends on the pinned agent version in [Dockerfile](Dockerfile). It is not a promise that every newer agent's authentication schema will be compatible. Provider login and refresh must be checked when upgrading. Settings, caches, and session databases are not saved by this sync.

## OpenCode 1.18.34 account compatibility

The pinned OpenCode version stores account credentials in its container-home SQLite database. AgentCage's embedded adapter reads and writes only the account credential fields and the selected account/organization from the `account` and `account_state` tables. It imports the allowlisted `.local/share/opencode/account.json` before the agent starts and exports that JSON after the agent exits. The database, session tables, logs, migration journal, and history remain in the disposable home; they are not copied to the host login store. Provider credentials in `.local/share/opencode/auth.json` use the existing sync unchanged.

For this pinned CLI, account commands are:

```bash
ac opencode -- console login
ac opencode -- console logout you@example.com
```

Replace `you@example.com` with the account to remove. These are OpenCode account operations; `ac login rm opencode` separately removes AgentCage's entire local OpenCode login store. Neither command is a promise of provider token revocation.

Synthetic database import/export and logout checks validate the adapter's local credential behavior, not an actual provider login or refresh. Real login may require provider/browser interaction and remains a separate validation step. If account sync reports an incompatible format/schema, use the pinned image and retry a fresh session; do not add the full database to the persistence allowlist.

This adapter is for normal disposable-home login sync. `--no-login` skips it, and `--env NAME` keeps the full home, including any OpenCode database, without this filtered sync.

## Concurrent sessions and local removal

Sessions snapshot the store without locking it for the whole agent run. At finalization:

- Unchanged staging does not replace a store changed by another session.
- Independent changes that conflict with a newer store fail visibly instead of silently restoring an older token.
- A conflicting valid snapshot is retained privately at the printed recovery path. It is never imported automatically; the accepted newer store stays unchanged.
- `login rm` invalidates older snapshots. A previously open session cannot restore the removed local store when it finishes.
- A newly started login-enabled session may create a new login after local removal.

A persistence failure is reported and gives a nonzero result. An already nonzero agent exit remains the primary process result. When a conflict occurs, keep the accepted store, finish overlapping sessions, and authenticate again in one fresh session if needed. Do not repeatedly overlap token refresh runs until recovery is complete.

The host accepts returned credentials only after the container completes the entire auth-file export. If a write or deletion fails midway, the accepted host snapshot is kept, the failure is reported, and the incomplete staging data is never imported as a login or logout. A cleanup failure reports any staging directory that could not be removed.

```bash
ac login list
ac login rm codex
```

`login list` shows recognizable saved local data, not confirmation that the provider accepts the token. `login rm` is local deletion, not provider revocation; running agents may already have copies. Revoke a token through its provider if that is required.

Local login removal does not delete credentials in persistent `--env` homes. Those homes are separate state.

## Disable login sync for a run

```bash
ac codex --no-login
```

The saved store is neither copied in nor updated by that run. This does not prevent the agent from editing the project or using the network. `--no-login` cannot be combined with `--env`.

## Full persistent homes

For a trusted workflow:

```bash
ac codex --env my-env
ac shell --env my-env
ac env list
ac env rm my-env
```

`--env NAME` keeps the entire container home under the current project's `.agentcage/envs/NAME/home`. Settings, caches, and credentials persist together; allowlist sync is not used. Finish sessions using the home before deleting it. Removing an environment deletes its local home contents, not provider tokens.

## Network and OAuth

Host networking allows localhost callback servers and access to the internet and host services. AgentCage does not block egress. If a workflow requires network filtering, configure it outside AgentCage.

Dedicated provider accounts and browser interaction are needed to verify OAuth and token refresh. Help/version checks and local token-file detection do not prove those flows.

## Limitations

Tokens are not encrypted. Container deletion does not erase project edits, the image, the login store, or persistent environment state. See [SECURITY.md](SECURITY.md).

# TODO: simple safe agent box

Goal: keep AgentCage simple and open-source-friendly: one current project mounted into a disposable rootless Podman container, no host home mount, no policy engine.

## Implemented now

- Short CLI command: install/build the binary as `ac`.
- Rootless Podman only; Docker is explicitly rejected.
- Current repository mounted at `/workspace/project`.
- Host `$HOME` is not mounted.
- Disposable tmpfs home by default.
- `.agentcage/` is hidden from the container by a tmpfs overlay at `/workspace/project/.agentcage`.
- Agent runs use dropped capabilities, `no-new-privileges`, a read-only image root, and CPU/memory/PID limits.
- `ac <agent> --no-login`: run without copying saved agent credentials into the container.
- Login persistence copies only allowlisted credential files and rejects symlinked/non-regular/oversized login files.
- Stale login staging cleanup: old `/tmp/agentcage-login-<agent>-*` directories owned by the current user, not owned by a live process, and older than 24 hours are removed on the next login-enabled run.
- Docs call out the real boundary: the agent can read/write the repo and can use network, but host `$HOME` is not mounted.

## Next high-value hardening

- Optional `--project-ro` mode for review/test runs where the agent may read the repo but not edit it.
- Optional egress control outside the main simple path, e.g. documented firewall/proxy recipes or a domain allowlist wrapper.
- Pin/verify third-party agent installers in the image build to reduce supply-chain drift.
- Optional per-project or throwaway login profiles for people who do not want one global login per host user.
- Add release checksums/signatures for binary packages.

## Intentionally out of scope

- Policy engine / `policy.json`
- Workspace diff staging/apply flow
- Secret scanning
- Session history/audit logs
- Desktop UI or dashboards

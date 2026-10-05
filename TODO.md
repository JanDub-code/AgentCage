# Maintenance scope

AgentCage keeps one current project mounted into a disposable rootless Podman container and runs one agent. Default homes are disposable; selected login files and optional full environment homes are explicit persistent state.

## Release checks

- Keep the pinned base digest, package snapshot, agent versions, checksums, and auth file contracts together when updating the image.
- Recheck OpenCode's account adapter against the pinned `account`/`account_state` schema; persist only account credential JSON and provider `auth.json`, never the session database or history through default login sync.
- Run the locked Cargo tests, clean shell completion probes, installer/package tests, and actual browser checks.
- Verify a clean Linux checkout or archive has LF shell scripts and executable entry points.
- Check package checksums, target, and source revision/state metadata before distribution.
- Run live containment and every declared agent's startup checks on the required Linux x86_64/rootless crun/systemd cgroups v2 baseline.
- Verify upstream OAuth and refresh with dedicated test accounts when auth contracts change.
- Validate each additional architecture or SELinux runtime before advertising it as tested.
- Keep README, tutorial, help, and web instructions aligned with the binary and image.
- Release signing is not implemented; current checksums detect corruption and do not authenticate the publisher.

## Intentionally out of scope

- Policy engine or `policy.json`
- Workspace diff, staging, or apply flows
- Secret scanning
- Session history or audit logs
- Desktop UI, multi-agent dashboards, or a new web framework

Network filtering and VM isolation are not promised. The writable project and available login tokens remain exposed to the selected agent; use throwaway projects and separate accounts when the workflow calls for them.

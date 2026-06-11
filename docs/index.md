# AgentCage

> **`ac`** — Run AI coding agents in a disposable rootless Podman container.

AgentCage mounts only the current directory, keeps your home and credentials out of reach, and auto-deletes the container when the agent exits. One command, one repo, no traces.

- [GitHub repository](https://github.com/JanDub-code/AgentCage)
- [README](https://github.com/JanDub-code/AgentCage/blob/main/README.md)
- [Security model](https://github.com/JanDub-code/AgentCage/blob/main/SECURITY.md)
- [Login persistence](https://github.com/JanDub-code/AgentCage/blob/main/LOGIN_PERSISTENCE.md)

## Quick start

```bash
ac codex     # run Codex CLI
ac claude    # run Claude Code
ac opencode  # run Opencode
ac antigravity  # run Google Antigravity CLI
ac shell     # open a contained shell
```

## Supported agents

| Command | Agent |
|---------|-------|
| `ac codex` | OpenAI Codex CLI |
| `ac claude` | Anthropic Claude Code |
| `ac opencode` | Opencode |
| `ac antigravity` | Google Antigravity CLI |

## License

MIT

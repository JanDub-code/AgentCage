# AgentCage

> **`ac`** — Run AI coding agents in a disposable rootless Podman container.

This is a tool I originally built for myself to try and test models and coding agents from providers I do not trust enough to run directly on my host machine.

AgentCage mounts only the current directory, keeps your home and credentials out of reach, and auto-deletes the container when the agent exits. One command, one repo, no traces.

- [GitHub repository](https://github.com/JanDub-code/AgentCage)
- [README](https://github.com/JanDub-code/AgentCage/blob/main/README.md)

Fedora is the primary tested target. The GitHub README includes installation, security boundaries, login persistence, and runtime bootstrap notes.

## Quick start

```bash
ac         # build/init the local runtime image
ac codex     # run Codex CLI
ac claude    # run Claude Code
ac opencode  # run Opencode
ac antigravity  # run Google Antigravity CLI
ac shell     # open a contained shell
```

First `ac` builds the local runtime image. First run of each agent downloads that specific agent lazily into the shared Podman cache.

## Supported agents

| Command | Agent |
|---------|-------|
| `ac codex` | OpenAI Codex CLI |
| `ac claude` | Anthropic Claude Code |
| `ac opencode` | Opencode |
| `ac antigravity` | Google Antigravity CLI |

## License

MIT

# AgentCage

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
![Platform](https://img.shields.io/badge/platform-linux%20|%20macOS-lightgrey)

> **`ac`** — Run AI coding agents in a disposable rootless Podman container.

A tool for running untrusted AI coding agents in isolation. Mounts only the current directory, keeps your home and credentials out of reach, and auto-deletes the container when the agent exits. One command, one repo, no traces.

---

## Quick start

```bash
ac                 # build/init the local runtime image
ac codex           # run Codex CLI
ac claude          # run Claude Code
ac opencode        # run Opencode
ac antigravity     # run Google Antigravity CLI
ac shell           # open a contained shell
```

First `ac` builds the local runtime image. First run of each agent downloads that specific agent lazily into the shared Podman cache.

---

## Supported agents

| Command            | Agent                       |
|--------------------|-----------------------------|
| `ac codex`         | OpenAI Codex CLI            |
| `ac claude`        | Anthropic Claude Code       |
| `ac opencode`      | Opencode                    |
| `ac antigravity`   | Google Antigravity CLI      |

---

## Resources

- [GitHub repository](https://github.com/JanDub-code/AgentCage)
- [README](https://github.com/JanDub-code/AgentCage/blob/main/README.md)

Fedora is the primary tested target. The GitHub README includes installation, security boundaries, login persistence, and runtime bootstrap notes.

---

## License

MIT

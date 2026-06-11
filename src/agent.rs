pub(crate) struct AgentInfo {
    pub name: &'static str,
    pub command: &'static str,
    pub install: AgentInstall,
}

pub(crate) enum AgentInstall {
    Npm(&'static str),
    Script(&'static str),
}

pub(crate) const SUPPORTED_AGENTS: &[AgentInfo] = &[
    AgentInfo {
        name: "codex",
        command: "codex",
        install: AgentInstall::Npm("@openai/codex"),
    },
    AgentInfo {
        name: "claude",
        command: "claude",
        install: AgentInstall::Npm("@anthropic-ai/claude-code"),
    },
    AgentInfo {
        name: "opencode",
        command: "opencode",
        install: AgentInstall::Npm("opencode-ai"),
    },
    AgentInfo {
        name: "antigravity",
        command: "agy",
        install: AgentInstall::Script(
            "curl -fsSL https://antigravity.google/cli/install.sh | HOME=/tmp/agy-install bash -s -- --dir /opt/agent-tools/bin",
        ),
    },
];

pub(crate) fn resolve(name: &str) -> Result<&'static AgentInfo, String> {
    SUPPORTED_AGENTS
        .iter()
        .find(|a| a.name == name)
        .ok_or_else(|| {
            let list: Vec<&str> = SUPPORTED_AGENTS.iter().map(|a| a.name).collect();
            format!("unknown agent '{name}'. try: {}", list.join(", "))
        })
}

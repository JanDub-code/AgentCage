pub(crate) const SUPPORTED_AGENTS: &[(&str, &str)] = &[
    ("codex", "codex"),
    ("claude", "claude"),
    ("opencode", "opencode"),
    ("antigravity", "agy"),
];

pub(crate) fn resolve(name: &str) -> Result<&'static str, String> {
    SUPPORTED_AGENTS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, command)| *command)
        .ok_or_else(|| {
            let list: Vec<&str> = SUPPORTED_AGENTS.iter().map(|(n, _)| *n).collect();
            format!("unknown agent '{name}'. try: {}", list.join(", "))
        })
}

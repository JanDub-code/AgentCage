use std::env;
use std::fs;
use std::io::{self, ErrorKind, Read};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::thread;
use std::time::Duration;

mod agent;

const IMAGE: &str = "agentcage/agent-tools:local";
const IMAGE_DOCKERFILE: &str = include_str!("../Dockerfile");
const CLI: &str = "ac";
const CONTAINER_PROJECT: &str = "/workspace/project";
const CONTAINER_CAGE_DIR: &str = "/workspace/project/.agentcage";
const CONTAINER_HOME: &str = "/workspace/session-home";
const PODMAN: &str = "podman";
const CAGE_DIR_NAME: &str = ".agentcage";
const ENVS_DIR_NAME: &str = "envs";
const LOGINS_DIR_NAME: &str = "logins";
const CONTAINER_LOGIN_SYNC: &str = "/workspace/login-sync";
const LOGIN_STAGING_PREFIX: &str = "agentcage-login-";
const LOGIN_STAGING_OWNER_FILE: &str = ".agentcage-owner-pid";
const STALE_LOGIN_STAGING_SECS: u64 = 24 * 60 * 60;
const MAX_LOGIN_FILE_BYTES: u64 = 10 * 1024 * 1024;

const LOGIN_SYNC_SCRIPT: &str = r#"
set -u

store="$1"
shift
max_bytes="$1"
shift
path_count="$1"
shift

paths=()
i=0
while [ "$i" -lt "$path_count" ]; do
    paths+=("$1")
    shift
    i=$((i + 1))
done

file_size_ok() {
    local path="$1"
    local rel="$2"
    local bytes
    bytes="$(wc -c < "$path" 2>/dev/null)" || return 1
    case "$bytes" in
        ''|*[!0-9]*) echo "agentcage: invalid login credential size: $rel" >&2; return 1 ;;
    esac
    if [ "$bytes" -gt "$max_bytes" ]; then
        echo "agentcage: login credential too large, not syncing: $rel" >&2
        return 1
    fi
    return 0
}

copy_private_file() {
    local src="$1"
    local dst="$2"
    local tmp="${dst}.agentcage-tmp.$$"
    rm -f "$tmp" 2>/dev/null || return 1
    cp "$src" "$tmp" || { rm -f "$tmp" 2>/dev/null || true; return 1; }
    chmod 0600 "$tmp" 2>/dev/null || true
    mv -f "$tmp" "$dst" || { rm -f "$tmp" 2>/dev/null || true; return 1; }
}

copy_in() {
    for rel in "${paths[@]}"; do
        case "$rel" in
            ""|/*|*..*) echo "agentcage: invalid login path: $rel" >&2; return 1 ;;
        esac
        src="$store/$rel"
        dst="$HOME/$rel"
        if [ -L "$src" ]; then
            echo "agentcage: refusing symlinked login credential: $rel" >&2
            return 1
        fi
        if [ -f "$src" ]; then
            file_size_ok "$src" "$rel" || return 1
            mkdir -p "$(dirname "$dst")" || return 1
            copy_private_file "$src" "$dst" || return 1
        fi
    done
}

copy_out() {
    local ok=0
    for rel in "${paths[@]}"; do
        src="$HOME/$rel"
        dst="$store/$rel"
        if [ -L "$src" ]; then
            echo "agentcage: warning: ignoring symlinked login credential: $rel" >&2
        elif [ -f "$src" ]; then
            if file_size_ok "$src" "$rel"; then
                mkdir -p "$(dirname "$dst")" || ok=1
                copy_private_file "$src" "$dst" || ok=1
            else
                ok=1
            fi
        elif [ -e "$src" ]; then
            echo "agentcage: warning: ignoring non-regular login credential: $rel" >&2
        else
            rm -f "$dst" 2>/dev/null || true
        fi
    done
    return "$ok"
}

copy_in || exit 1
"$@"
status=$?
copy_out || echo "agentcage: warning: failed to save login credentials" >&2
exit "$status"
"#;

const RUNTIME_ENV_VARS: &[(&str, &str)] = &[
    ("HOME", CONTAINER_HOME),
    ("CODEX_HOME", CONTAINER_HOME),
    ("USER", "agentcage"),
    ("LOGNAME", "agentcage"),
    ("SHELL", "/bin/bash"),
    ("XDG_CONFIG_HOME", "/workspace/session-home/.config"),
    ("XDG_CACHE_HOME", "/workspace/session-home/.cache"),
    ("XDG_DATA_HOME", "/workspace/session-home/.local/share"),
    ("XDG_STATE_HOME", "/workspace/session-home/.local/state"),
    ("NPM_CONFIG_CACHE", "/workspace/session-home/.npm"),
    ("BUN_INSTALL", "/workspace/session-home/.bun"),
];

pub fn run() {
    let args: Vec<String> = env::args().skip(1).collect();
    match dispatch(&args) {
        Ok(()) => {}
        Err(e) => {
            eprintln!("error: {e}");
            process::exit(1);
        }
    }
}

enum Cmd {
    Init,
    Agent {
        name: String,
        env: Option<String>,
        login: LoginPolicy,
        args: Vec<String>,
    },
    Shell {
        env: Option<String>,
    },
    Clean,
    EnvList,
    EnvRemove {
        name: String,
    },
    LoginList,
    LoginRemove {
        agent: String,
    },
    Help,
    Completions {
        shell: String,
    },
}

enum HomeMount {
    Disposable,
    Persistent { name: String, host_path: PathBuf },
}

enum LoginMount {
    None,
    Disabled,
    Persistent {
        agent: String,
        host_path: PathBuf,
        staging_path: PathBuf,
        paths: &'static [&'static str],
        has_saved: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginPolicy {
    Auto,
    Disabled,
}

fn dispatch(args: &[String]) -> Result<(), String> {
    let cmd = parse(args)?;
    let cwd = env::current_dir().map_err(|e| format!("cannot read current directory: {e}"))?;
    match cmd {
        Cmd::Init => cmd_init(&cwd),
        Cmd::Agent {
            name,
            env,
            login,
            args: extra,
        } => cmd_agent(&cwd, &name, env.as_deref(), login, &extra),
        Cmd::Shell { env } => cmd_shell(&cwd, env.as_deref()),
        Cmd::Clean => cmd_clean(&cwd),
        Cmd::EnvList => cmd_env_list(&cwd),
        Cmd::EnvRemove { name } => cmd_env_remove(&cwd, &name),
        Cmd::LoginList => cmd_login_list(),
        Cmd::LoginRemove { agent } => cmd_login_remove(&agent),
        Cmd::Help => {
            print_help();
            Ok(())
        }
        Cmd::Completions { shell } => {
            print_completions(&shell);
            Ok(())
        }
    }
}

fn parse(args: &[String]) -> Result<Cmd, String> {
    let rest = strip_global_flags(args)?;
    let Some(cmd) = rest.first() else {
        return Ok(Cmd::Init);
    };

    match cmd.as_str() {
        "init" => {
            if has_help_arg(&rest[1..]) {
                return Ok(Cmd::Help);
            }
            expect_no_args(rest, "init")?;
            Ok(Cmd::Init)
        }
        "codex" | "claude" | "opencode" | "antigravity" => {
            let name = cmd.clone();
            let mut env = None;
            let mut login = LoginPolicy::Auto;
            let mut extra = Vec::new();
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--env" => {
                        let value = rest
                            .get(i + 1)
                            .ok_or_else(|| "--env needs a value".to_string())?;
                        validate_env_name(value)?;
                        if env.is_some() {
                            return Err("duplicate --env option".into());
                        }
                        env = Some(value.clone());
                        i += 1;
                    }
                    "--no-login" => {
                        login = LoginPolicy::Disabled;
                    }
                    "-h" | "--help" => return Ok(Cmd::Help),
                    "--" => {
                        extra = rest[i + 1..].to_vec();
                        break;
                    }
                    arg if arg.starts_with('-') => return Err(format!("unknown option '{arg}'")),
                    arg => extra.push(arg.to_string()),
                }
                i += 1;
            }
            Ok(Cmd::Agent {
                name,
                env,
                login,
                args: extra,
            })
        }
        "shell" => {
            let mut env = None;
            let mut i = 1;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--env" => {
                        let value = rest
                            .get(i + 1)
                            .ok_or_else(|| "--env needs a value".to_string())?;
                        validate_env_name(value)?;
                        if env.is_some() {
                            return Err("duplicate --env option".into());
                        }
                        env = Some(value.clone());
                        i += 1;
                    }
                    "-h" | "--help" => return Ok(Cmd::Help),
                    arg => return Err(format!("unexpected argument '{arg}'")),
                }
                i += 1;
            }
            Ok(Cmd::Shell { env })
        }
        "clean" => {
            if has_help_arg(&rest[1..]) {
                return Ok(Cmd::Help);
            }
            expect_no_args(rest, "clean")?;
            Ok(Cmd::Clean)
        }
        "env" => parse_env_command(rest),
        "login" => parse_login_command(rest),
        "remove" => parse_remove_command(rest),
        "help" | "-h" | "--help" => Ok(Cmd::Help),
        "completions" => {
            let Some(shell) = rest.get(1) else {
                return Err(format!("usage: {CLI} completions <bash|zsh|fish>"));
            };
            if rest.len() != 2 || !matches!(shell.as_str(), "bash" | "zsh" | "fish") {
                return Err(format!("usage: {CLI} completions <bash|zsh|fish>"));
            }
            Ok(Cmd::Completions {
                shell: shell.clone(),
            })
        }
        unknown => {
            let hint = "init, codex, claude, opencode, antigravity, shell, clean, env, login, help";
            Err(format!("unknown command '{unknown}'. try: {hint}"))
        }
    }
}

fn parse_env_command(args: &[String]) -> Result<Cmd, String> {
    if args.len() < 2 {
        return Err(format!("usage: {CLI} env <list|rm NAME>"));
    }
    match args[1].as_str() {
        "list" => {
            if args.len() != 2 {
                return Err(format!("usage: {CLI} env list"));
            }
            Ok(Cmd::EnvList)
        }
        "rm" | "remove" | "delete" => {
            let Some(name) = args.get(2) else {
                return Err(format!("usage: {CLI} env rm <name>"));
            };
            if args.len() != 3 {
                return Err(format!("usage: {CLI} env rm <name>"));
            }
            validate_env_name(name)?;
            Ok(Cmd::EnvRemove { name: name.clone() })
        }
        "-h" | "--help" => Ok(Cmd::Help),
        other => Err(format!(
            "unknown env subcommand '{other}'. try: list, rm <name>"
        )),
    }
}

fn parse_login_command(args: &[String]) -> Result<Cmd, String> {
    if args.len() < 2 {
        return Err(format!("usage: {CLI} login <list|rm AGENT>"));
    }
    match args[1].as_str() {
        "list" => {
            if args.len() != 2 {
                return Err(format!("usage: {CLI} login list"));
            }
            Ok(Cmd::LoginList)
        }
        "rm" | "remove" | "delete" => {
            let Some(agent) = args.get(2) else {
                return Err(format!("usage: {CLI} login rm <agent>"));
            };
            if args.len() != 3 {
                return Err(format!("usage: {CLI} login rm <agent>"));
            }
            validate_login_agent(agent)?;
            Ok(Cmd::LoginRemove {
                agent: agent.clone(),
            })
        }
        "-h" | "--help" => Ok(Cmd::Help),
        other => Err(format!(
            "unknown login subcommand '{other}'. try: list, rm <agent>"
        )),
    }
}

fn parse_remove_command(args: &[String]) -> Result<Cmd, String> {
    if args.len() < 2 {
        return Err(format!("usage: {CLI} remove <env NAME|login AGENT>"));
    }
    match args[1].as_str() {
        "env" => {
            let Some(name) = args.get(2) else {
                return Err(format!("usage: {CLI} remove env <name>"));
            };
            if args.len() != 3 {
                return Err(format!("usage: {CLI} remove env <name>"));
            }
            validate_env_name(name)?;
            Ok(Cmd::EnvRemove { name: name.clone() })
        }
        "login" => {
            let Some(agent) = args.get(2) else {
                return Err(format!("usage: {CLI} remove login <agent>"));
            };
            if args.len() != 3 {
                return Err(format!("usage: {CLI} remove login <agent>"));
            }
            validate_login_agent(agent)?;
            Ok(Cmd::LoginRemove {
                agent: agent.clone(),
            })
        }
        other => Err(format!(
            "unknown remove target '{other}'. try: {CLI} remove env <name> or {CLI} remove login <agent>"
        )),
    }
}

fn strip_global_flags(args: &[String]) -> Result<&[String], String> {
    let Some(first) = args.first() else {
        return Ok(args);
    };
    match first.as_str() {
        "-h" | "--help" => Ok(args),
        arg if arg.starts_with('-') => Err(format!("unknown global option '{arg}'")),
        _ => Ok(args),
    }
}

fn has_help_arg(args: &[String]) -> bool {
    args.iter().any(|arg| arg == "-h" || arg == "--help")
}

fn expect_no_args(args: &[String], cmd: &str) -> Result<(), String> {
    for arg in &args[1..] {
        match arg.as_str() {
            "-h" | "--help" => return Ok(()),
            _ => return Err(format!("'{cmd}' takes no arguments, got '{arg}'")),
        }
    }
    Ok(())
}

fn validate_env_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("environment name cannot be empty".into());
    }
    if name == "." || name == ".." {
        return Err("environment name cannot be '.' or '..'".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err("invalid environment name. use only letters, numbers, '-', '_' or '.'".into());
    }
    Ok(())
}

fn validate_login_agent(name: &str) -> Result<(), String> {
    agent::resolve(name).map(|_| ())?;
    if login_paths_for_agent(name).is_empty() {
        return Err(format!(
            "login persistence is not configured for '{name}' yet"
        ));
    }
    Ok(())
}

fn cmd_init(cwd: &Path) -> Result<(), String> {
    let podman = detect_podman()?;

    ensure_cage_dir(cwd)?;
    ensure_runtime_image(&podman)?;

    println!("AgentCage — ready");
    println!("  image:  {}", IMAGE);
    println!("  runtime: {} {}", podman.executable, podman.version);
    println!(
        "  rootless: {}",
        if podman.rootless {
            "yes"
        } else {
            "no (expected)"
        }
    );
    println!();
    println!("Commands:");
    println!("  {CLI} codex       run Codex CLI in a disposable cage");
    println!("  {CLI} claude      run Claude Code in a disposable cage");
    println!("  {CLI} opencode    run Opencode in a disposable cage");
    println!("  {CLI} antigravity run Google Antigravity CLI in a disposable cage");
    println!("  {CLI} shell       open a shell inside the cage");
    println!("  {CLI} env list    list persistent environments");
    println!("  {CLI} env rm NAME remove a persistent environment");
    println!("  {CLI} login list  list saved login credentials");
    println!("  {CLI} login rm AGENT remove saved login credentials");
    println!("  {CLI} clean       remove container artifacts");
    println!();
    println!("Tab completion:");
    println!("  source <({CLI} completions bash)");
    println!("  source <({CLI} completions zsh)");
    println!("  {CLI} completions fish | source");

    Ok(())
}

fn cmd_agent(
    cwd: &Path,
    agent_name: &str,
    env_name: Option<&str>,
    login_policy: LoginPolicy,
    extra_args: &[String],
) -> Result<(), String> {
    ensure_cage_dir(cwd)?;
    if matches!(login_policy, LoginPolicy::Disabled) && env_name.is_some() {
        return Err("--no-login cannot be combined with --env because --env reuses a full persistent home that may already contain credentials".into());
    }

    let agent_info = agent::resolve(agent_name)?;
    let podman = detect_podman()?;
    ensure_runtime_image(&podman)?;
    ensure_tool_cache_volume(&podman)?;

    let home_mount = resolve_home_mount(cwd, env_name)?;
    let login_mount = resolve_login_mount(agent_name, &home_mount, login_policy)?;
    print_status(cwd, &home_mount, &login_mount);
    print_agent_startup_hints(&podman, agent_info, &login_mount);
    let result = (|| {
        let (argv, container_name) = build_run_argv(
            cwd,
            Some(agent_info),
            extra_args,
            false,
            &home_mount,
            &login_mount,
        )?;
        run_podman(&argv, &container_name, &login_mount)
    })();
    if let Err(e) = result {
        cleanup_login_staging(&login_mount);
        let _ = harden_persistent_home(&home_mount);
        return Err(e);
    }
    harden_persistent_home(&home_mount)
}

fn cmd_shell(cwd: &Path, env_name: Option<&str>) -> Result<(), String> {
    ensure_cage_dir(cwd)?;
    let podman = detect_podman()?;
    ensure_runtime_image(&podman)?;
    ensure_tool_cache_volume(&podman)?;
    let home_mount = resolve_home_mount(cwd, env_name)?;
    let login_mount = LoginMount::None;
    print_status(cwd, &home_mount, &login_mount);
    let (argv, container_name) = build_run_argv(cwd, None, &[], true, &home_mount, &login_mount)?;
    run_podman(&argv, &container_name, &login_mount)?;
    harden_persistent_home(&home_mount)
}

fn cmd_clean(_cwd: &Path) -> Result<(), String> {
    let podman = detect_podman()?;
    let output = Command::new(&podman.executable)
        .args([
            "ps",
            "-a",
            "--filter",
            "name=agentcage",
            "--format",
            "{{.Names}}",
        ])
        .output()
        .map_err(|e| format!("podman ps failed: {e}"))?;
    let names = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<&str> = names.lines().filter(|l| !l.is_empty()).collect();
    if containers.is_empty() {
        println!("AgentCage — nothing to clean");
        println!();
        println!("note: installed agents are cached in the 'agentcage-tools' podman volume.");
        println!("to remove it: podman volume rm agentcage-tools");
        return Ok(());
    }
    for name in containers {
        println!("removing container: {name}");
        let _ = Command::new(&podman.executable)
            .args(["rm", "-f", name])
            .status();
    }
    println!("done");
    println!();
    println!("note: installed agents are cached in the 'agentcage-tools' podman volume.");
    println!("to remove it: podman volume rm agentcage-tools");
    Ok(())
}

fn cmd_env_list(cwd: &Path) -> Result<(), String> {
    let envs_dir = ensure_cage_dir(cwd)?.join(ENVS_DIR_NAME);
    let entries = match fs::read_dir(&envs_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            println!("no persistent environments");
            return Ok(());
        }
        Err(e) => {
            return Err(format!(
                "cannot list environments in {}: {e}",
                envs_dir.display()
            ))
        }
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read env entry: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot inspect env entry: {e}"))?;
        if !kind.is_dir() {
            continue;
        }
        names.push(entry.file_name().to_string_lossy().to_string());
    }
    names.sort();

    if names.is_empty() {
        println!("no persistent environments");
        return Ok(());
    }

    println!("persistent environments:");
    for name in names {
        println!("  {name}");
    }
    Ok(())
}

fn cmd_env_remove(cwd: &Path, name: &str) -> Result<(), String> {
    validate_env_name(name)?;
    let env_dir = ensure_cage_dir(cwd)?.join(ENVS_DIR_NAME).join(name);
    match fs::symlink_metadata(&env_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to remove symlinked environment {}",
                env_dir.display()
            ));
        }
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "environment path exists but is not a directory: {}",
                env_dir.display()
            ));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(format!("environment '{name}' does not exist"));
        }
        Err(e) => {
            return Err(format!(
                "cannot inspect environment '{}': {e}",
                env_dir.display()
            ));
        }
    }
    match fs::remove_dir_all(&env_dir) {
        Ok(()) => {
            println!("removed environment: {name}");
            Ok(())
        }
        Err(e) => Err(format!(
            "cannot remove environment '{}': {e}",
            env_dir.display()
        )),
    }
}

fn cmd_login_list() -> Result<(), String> {
    let logins_dir = match logins_dir() {
        Ok(path) => path,
        Err(e) => return Err(e),
    };
    let entries = match fs::read_dir(&logins_dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => {
            println!("no saved logins");
            return Ok(());
        }
        Err(e) => {
            return Err(format!(
                "cannot list saved logins in {}: {e}",
                logins_dir.display()
            ))
        }
    };

    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot read login entry: {e}"))?;
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot inspect login entry: {e}"))?;
        if !kind.is_dir() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if login_paths_for_agent(&name).is_empty() {
            continue;
        }
        if login_store_has_files(&name, &entry.path(), login_paths_for_agent(&name)) {
            names.push(name);
        }
    }
    names.sort();

    if names.is_empty() {
        println!("no saved logins");
        return Ok(());
    }

    println!("saved logins:");
    for name in names {
        println!("  {name}");
    }
    Ok(())
}

fn cmd_login_remove(agent_name: &str) -> Result<(), String> {
    validate_login_agent(agent_name)?;
    let login_dir = login_dir(agent_name)?;
    match fs::symlink_metadata(&login_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to remove symlinked saved login {}",
                login_dir.display()
            ));
        }
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "saved login path exists but is not a directory: {}",
                login_dir.display()
            ));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            return Err(format!("saved login for '{agent_name}' does not exist"));
        }
        Err(e) => {
            return Err(format!(
                "cannot inspect saved login '{}': {e}",
                login_dir.display()
            ));
        }
    }
    match fs::remove_dir_all(&login_dir) {
        Ok(()) => {
            println!("removed saved login: {agent_name}");
            Ok(())
        }
        Err(e) => Err(format!(
            "cannot remove saved login '{}': {e}",
            login_dir.display()
        )),
    }
}

fn print_status(cwd: &Path, home_mount: &HomeMount, login_mount: &LoginMount) {
    let network = "on (host localhost available for OAuth callbacks)";
    let home_line = match home_mount {
        HomeMount::Disposable => "disposable tmpfs".to_string(),
        HomeMount::Persistent { name, host_path } => {
            format!("persistent env '{name}' ({})", host_path.display())
        }
    };
    let login_line = match (home_mount, login_mount) {
        (HomeMount::Persistent { .. }, _) => "included in persistent home".to_string(),
        (_, LoginMount::Disabled) => "disabled (--no-login)".to_string(),
        (
            _,
            LoginMount::Persistent {
                agent,
                host_path,
                has_saved,
                ..
            },
        ) => {
            if *has_saved {
                format!("saved {agent} auth only ({})", host_path.display())
            } else {
                format!(
                    "first {agent} login will be saved ({})",
                    host_path.display()
                )
            }
        }
        _ => "disposable".to_string(),
    };
    let host_home_line = match login_mount {
        LoginMount::Persistent { .. } => "not mounted; login sync uses a filtered temp bind",
        LoginMount::None | LoginMount::Disabled => "not mounted",
    };
    println!("AgentCage");
    println!("  Workspace:  {} -> {}", cwd.display(), CONTAINER_PROJECT);
    println!("  Home:       {home_line}");
    println!("  Login:      {login_line}");
    println!("  Host home:  {host_home_line}");
    println!("  Network:    {network}");
    println!("  Container:  auto-delete on exit");
    println!();
}

fn print_agent_startup_hints(
    podman: &PodmanInfo,
    agent_info: &agent::AgentInfo,
    login_mount: &LoginMount,
) {
    let mut printed = false;

    match tool_cache_status(podman, agent_info) {
        Ok(false) => {
            println!(
                "Setup: first {} run will download/install {} into the shared Podman tool cache.",
                agent_info.name, agent_info.command
            );
            printed = true;
        }
        Ok(true) => {}
        Err(e) => {
            eprintln!("warning: cannot inspect cached agent tools: {e}");
        }
    }

    if let LoginMount::Persistent {
        agent,
        has_saved: false,
        ..
    } = login_mount
    {
        println!(
            "Login: no saved {agent} credentials yet; if a browser login opens, finish it and return here."
        );
        printed = true;
    }

    if printed {
        println!();
    }
}

fn ensure_cage_dir(cwd: &Path) -> Result<PathBuf, String> {
    let cage_dir = cwd.join(CAGE_DIR_NAME);
    match fs::symlink_metadata(&cage_dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!(
                "refusing to use symlinked project state directory {}",
                cage_dir.display()
            ));
        }
        Ok(meta) if meta.file_type().is_dir() => {}
        Ok(_) => {
            return Err(format!(
                "project state path exists but is not a directory: {}",
                cage_dir.display()
            ));
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {}
        Err(e) => {
            return Err(format!(
                "cannot inspect project state directory {}: {e}",
                cage_dir.display()
            ));
        }
    }

    create_private_dir_all(&cage_dir)?;
    cage_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve project state directory: {e}"))
}

fn agentcage_data_dir() -> Result<PathBuf, String> {
    if let Ok(value) = env::var("XDG_DATA_HOME") {
        if !value.is_empty() {
            return Ok(PathBuf::from(value).join("agentcage"));
        }
    }

    let home = env::var("HOME").map_err(|_| {
        "cannot determine host data directory. set XDG_DATA_HOME or HOME".to_string()
    })?;
    if home.is_empty() {
        return Err("cannot determine host data directory. set XDG_DATA_HOME or HOME".into());
    }
    Ok(PathBuf::from(home)
        .join(".local")
        .join("share")
        .join("agentcage"))
}

fn logins_dir() -> Result<PathBuf, String> {
    Ok(agentcage_data_dir()?.join(LOGINS_DIR_NAME))
}

fn login_dir(agent_name: &str) -> Result<PathBuf, String> {
    Ok(logins_dir()?.join(agent_name))
}

fn resolve_home_mount(cwd: &Path, env_name: Option<&str>) -> Result<HomeMount, String> {
    let Some(name) = env_name else {
        return Ok(HomeMount::Disposable);
    };
    validate_env_name(name)?;
    let cage_dir = ensure_cage_dir(cwd)?;
    let env_home = Path::new(ENVS_DIR_NAME).join(name).join("home");
    let home_path = create_private_subdir_all(&cage_dir, &env_home)?;
    harden_known_login_files_in_home(&home_path)?;
    Ok(HomeMount::Persistent {
        name: name.to_string(),
        host_path: home_path,
    })
}

fn resolve_login_mount(
    agent_name: &str,
    home_mount: &HomeMount,
    login_policy: LoginPolicy,
) -> Result<LoginMount, String> {
    if !matches!(home_mount, HomeMount::Disposable) {
        return Ok(LoginMount::None);
    }

    if matches!(login_policy, LoginPolicy::Disabled) {
        return Ok(LoginMount::Disabled);
    }

    let paths = login_paths_for_agent(agent_name);
    if paths.is_empty() {
        return Ok(LoginMount::None);
    }

    cleanup_stale_login_staging(agent_name);

    let host_path = ensure_logins_dir()?.join(agent_name);
    create_private_dir_all(&host_path)?;
    harden_login_store(agent_name, &host_path, paths)?;
    let has_saved = login_store_has_files(agent_name, &host_path, paths);

    let staging_path = create_private_temp_dir(&format!("{LOGIN_STAGING_PREFIX}{agent_name}-"))?;
    write_login_staging_owner(&staging_path)?;
    if let Err(e) = copy_login_files(agent_name, &host_path, &staging_path, paths) {
        let _ = fs::remove_dir_all(&staging_path);
        return Err(e);
    }

    Ok(LoginMount::Persistent {
        agent: agent_name.to_string(),
        host_path,
        staging_path,
        paths,
        has_saved,
    })
}

fn login_paths_for_agent(agent_name: &str) -> &'static [&'static str] {
    match agent_name {
        // With CODEX_HOME set to CONTAINER_HOME, current Codex writes auth.json
        // directly under /workspace/session-home. Keep ~/.codex/auth.json too so
        // saved credentials survive if Codex switches back to its default home.
        "codex" => &["auth.json", ".codex/auth.json"],
        // Claude Code stores OAuth tokens in ~/.claude/.credentials.json and
        // API-key auth in ~/.claude.json.
        "claude" => &[".claude/.credentials.json", ".claude.json"],
        // Opencode stores provider credentials in auth.json and account/session
        // state in account.json under its XDG data directory.
        "opencode" => &[
            ".local/share/opencode/auth.json",
            ".local/share/opencode/account.json",
        ],
        // Antigravity CLI uses the OS keyring when available. In the cage,
        // Linux Secret Service/dbus is absent, so agy falls back to this
        // token file under its Gemini app-data directory.
        "antigravity" => &[".gemini/antigravity-cli/antigravity-oauth-token"],
        _ => &[],
    }
}

fn login_store_has_files(agent_name: &str, dir: &Path, paths: &[&str]) -> bool {
    paths.iter().any(|rel| {
        let path = dir.join(rel);
        matches!(
            fs::symlink_metadata(&path),
            Ok(meta) if meta.file_type().is_file()
        ) && ensure_login_file_size(&path).is_ok()
            && should_persist_login_file(agent_name, rel, &path).unwrap_or(false)
    })
}

fn create_private_temp_dir(prefix: &str) -> Result<PathBuf, String> {
    let tmp = env::temp_dir();
    for _ in 0..100 {
        let path = tmp.join(format!("{prefix}{}", nanoid(12)));
        match fs::DirBuilder::new().mode(0o700).create(&path) {
            Ok(()) => {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                    .map_err(|e| format!("cannot set permissions on {}: {e}", path.display()))?;
                return Ok(path);
            }
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(format!(
                    "cannot create private temporary directory {}: {e}",
                    path.display()
                ));
            }
        }
    }
    Err(format!(
        "cannot create unique private temporary directory in {}",
        tmp.display()
    ))
}

fn create_private_dir_all(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|e| format!("cannot create private directory {}: {e}", path.display()))?;
    let meta = fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect private directory {}: {e}", path.display()))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "refusing to use symlinked private directory {}",
            path.display()
        ));
    }
    if !meta.file_type().is_dir() {
        return Err(format!(
            "private path exists but is not a directory: {}",
            path.display()
        ));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot set permissions on {}: {e}", path.display()))?;
    Ok(())
}

fn create_private_subdir_all(root: &Path, rel: &Path) -> Result<PathBuf, String> {
    create_private_dir_all(root)?;

    let mut current = root.to_path_buf();
    for component in rel.components() {
        match component {
            Component::Normal(part) => {
                current.push(part);
                create_private_dir_all(&current)?;
            }
            Component::CurDir => {}
            _ => {
                return Err(format!(
                    "invalid private subdirectory path {}",
                    rel.display()
                ))
            }
        }
    }

    Ok(current)
}

fn create_private_parent_for_rel(root: &Path, rel: &str) -> Result<(), String> {
    let rel_path = Path::new(rel);
    if let Some(parent) = rel_path.parent() {
        create_private_subdir_all(root, parent)?;
    } else {
        create_private_dir_all(root)?;
    }
    Ok(())
}

fn copy_private_file_atomic(src: &Path, dst: &Path) -> Result<(), String> {
    ensure_login_file_size(src)?;
    let parent = dst
        .parent()
        .ok_or_else(|| format!("cannot determine parent for {}", dst.display()))?;
    create_private_dir_all(parent)?;

    let file_name = dst
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("invalid destination file name {}", dst.display()))?;

    for _ in 0..100 {
        let tmp = parent.join(format!(
            ".{file_name}.agentcage-tmp-{}-{}",
            process::id(),
            nanoid(8)
        ));
        if tmp.exists() {
            continue;
        }

        let copy_result = fs::copy(src, &tmp)
            .map_err(|e| format!("cannot copy {} to {}: {e}", src.display(), tmp.display()))
            .and_then(|_| {
                fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("cannot set permissions on {}: {e}", tmp.display()))
            })
            .and_then(|_| {
                fs::rename(&tmp, dst)
                    .map_err(|e| format!("cannot move {} to {}: {e}", tmp.display(), dst.display()))
            });

        if let Err(e) = copy_result {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        return Ok(());
    }

    Err(format!(
        "cannot create unique temporary credential file near {}",
        dst.display()
    ))
}

fn ensure_logins_dir() -> Result<PathBuf, String> {
    let data_dir = agentcage_data_dir()?;
    create_private_dir_all(&data_dir)?;
    let logins = data_dir.join(LOGINS_DIR_NAME);
    create_private_dir_all(&logins)?;
    Ok(logins)
}

fn harden_login_store(agent_name: &str, root: &Path, paths: &[&str]) -> Result<(), String> {
    create_private_dir_all(root)?;
    for rel in paths {
        let path = root.join(rel);
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_file() => {
                create_private_parent_for_rel(root, rel)?;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("cannot set permissions on {}: {e}", path.display()))?;
                ensure_login_file_size(&path)?;
                if !should_persist_login_file(agent_name, rel, &path)? {
                    continue;
                }
            }
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to use symlinked login credential {}",
                    path.display()
                ));
            }
            Ok(_) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "cannot inspect login credential {}: {e}",
                    path.display()
                ))
            }
        }
    }
    Ok(())
}

fn harden_persistent_home(home_mount: &HomeMount) -> Result<(), String> {
    let HomeMount::Persistent { host_path, .. } = home_mount else {
        return Ok(());
    };

    create_private_dir_all(host_path)?;
    harden_known_login_files_in_home(host_path)
}

fn harden_known_login_files_in_home(home_path: &Path) -> Result<(), String> {
    for agent in agent::SUPPORTED_AGENTS {
        let agent_name = agent.name;
        for rel in login_paths_for_agent(agent_name) {
            let path = home_path.join(rel);
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_file() => {
                    create_private_parent_for_rel(home_path, rel)?;
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).map_err(|e| {
                        format!("cannot set permissions on {}: {e}", path.display())
                    })?;
                }
                Ok(meta) if meta.file_type().is_symlink() => {
                    return Err(format!(
                        "refusing to use symlinked login credential {}",
                        path.display()
                    ));
                }
                Ok(_) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => {
                    return Err(format!(
                        "cannot inspect login credential {}: {e}",
                        path.display()
                    ))
                }
            }
        }
    }
    Ok(())
}

fn copy_login_files(
    agent_name: &str,
    from: &Path,
    to: &Path,
    paths: &[&str],
) -> Result<(), String> {
    for rel in paths {
        let src = from.join(rel);
        match fs::symlink_metadata(&src) {
            Ok(meta) if meta.file_type().is_file() => {
                ensure_login_file_size(&src)?;
                if !should_persist_login_file(agent_name, rel, &src)? {
                    continue;
                }
                let dst = to.join(rel);
                create_private_parent_for_rel(to, rel)?;
                copy_private_file_atomic(&src, &dst)?;
            }
            Ok(meta) if meta.file_type().is_symlink() => {
                return Err(format!(
                    "refusing to use symlinked login credential {}",
                    src.display()
                ));
            }
            Ok(meta) if meta.file_type().is_dir() => {
                return Err(format!(
                    "refusing to use directory as login credential {}",
                    src.display()
                ));
            }
            Ok(_) => {
                return Err(format!(
                    "refusing to use non-regular login credential {}",
                    src.display()
                ));
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "cannot inspect login credential {}: {e}",
                    src.display()
                ))
            }
        }
    }
    Ok(())
}

fn should_persist_login_file(agent_name: &str, rel: &str, path: &Path) -> Result<bool, String> {
    match (agent_name, rel) {
        ("claude", ".claude.json") => {
            let content = read_login_text(path)?;
            Ok(json_key_has_value(&content, "primaryApiKey"))
        }
        ("claude", ".claude/.credentials.json") => {
            let content = read_login_text(path)?;
            Ok(json_key_has_value(&content, "claudeAiOauth"))
        }
        ("opencode", ".local/share/opencode/auth.json") => {
            let content = read_login_text(path)?;
            Ok(nonempty_jsonish(&content))
        }
        ("opencode", ".local/share/opencode/account.json") => {
            let content = read_login_text(path)?;
            Ok(json_key_has_value(&content, "accounts"))
        }
        ("antigravity", ".gemini/antigravity-cli/antigravity-oauth-token") => nonempty_file(path),
        _ => Ok(true),
    }
}

fn read_login_text(path: &Path) -> Result<String, String> {
    ensure_login_file_size(path)?;
    fs::read_to_string(path)
        .map_err(|e| format!("cannot read login credential {}: {e}", path.display()))
}

fn ensure_login_file_size(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect login credential {}: {e}", path.display()))?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "refusing to use symlinked login credential {}",
            path.display()
        ));
    }
    if !metadata.file_type().is_file() {
        return Err(format!(
            "refusing to use non-regular login credential {}",
            path.display()
        ));
    }
    if metadata.len() > MAX_LOGIN_FILE_BYTES {
        return Err(format!(
            "refusing oversized login credential {} ({} bytes; max {} bytes)",
            path.display(),
            metadata.len(),
            MAX_LOGIN_FILE_BYTES
        ));
    }
    Ok(())
}

fn nonempty_jsonish(content: &str) -> bool {
    let compact: String = content.chars().filter(|c| !c.is_whitespace()).collect();
    !compact.is_empty() && compact != "{}" && compact != "[]"
}

fn nonempty_file(path: &Path) -> Result<bool, String> {
    ensure_login_file_size(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("cannot inspect login credential {}: {e}", path.display()))?;
    Ok(metadata.len() > 0)
}

fn json_key_has_value(content: &str, key: &str) -> bool {
    let needle = format!("\"{key}\"");
    let mut offset = 0;
    while let Some(relative) = content[offset..].find(&needle) {
        let after_key = offset + relative + needle.len();
        let Some(colon) = content[after_key..].find(':') else {
            return false;
        };
        let value = content[after_key + colon + 1..].trim_start();
        if !value.starts_with("null")
            && !value.starts_with("\"\"")
            && !value.starts_with("{}")
            && !value.starts_with("[]")
        {
            return true;
        }
        offset = after_key;
    }
    false
}

fn finalize_login_mount(login_mount: &LoginMount) -> Result<(), String> {
    let LoginMount::Persistent {
        agent,
        host_path,
        staging_path,
        paths,
        ..
    } = login_mount
    else {
        return Ok(());
    };

    let mut first_error = None;

    for rel in *paths {
        let src = staging_path.join(rel);
        let dst = host_path.join(rel);
        match fs::symlink_metadata(&src) {
            Ok(meta) if meta.file_type().is_file() => {
                if let Err(e) = ensure_login_file_size(&src) {
                    first_error.get_or_insert(e);
                    continue;
                }
                match should_persist_login_file(agent, rel, &src) {
                    Ok(true) => {}
                    Ok(false) => {
                        let _ = fs::remove_file(&dst);
                        continue;
                    }
                    Err(e) => {
                        first_error.get_or_insert(e);
                        continue;
                    }
                }
                if let Err(e) = create_private_parent_for_rel(host_path, rel) {
                    first_error.get_or_insert(e);
                    continue;
                }
                if let Err(e) = copy_private_file_atomic(&src, &dst) {
                    first_error.get_or_insert(e);
                }
            }
            Ok(_) => {
                eprintln!(
                    "warning: ignoring non-regular login credential {}",
                    src.display()
                );
                let _ = fs::remove_file(&dst);
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                let _ = fs::remove_file(&dst);
            }
            Err(e) => {
                first_error.get_or_insert(format!(
                    "cannot inspect login credential {}: {e}",
                    src.display()
                ));
            }
        }
    }

    cleanup_login_staging(login_mount);

    if let Some(e) = first_error {
        Err(e)
    } else {
        Ok(())
    }
}

fn cleanup_login_staging(login_mount: &LoginMount) {
    if let LoginMount::Persistent { staging_path, .. } = login_mount {
        let _ = fs::remove_dir_all(staging_path);
    }
}

fn cleanup_stale_login_staging(agent_name: &str) {
    if let Err(e) = cleanup_stale_login_staging_in(
        &env::temp_dir(),
        agent_name,
        Duration::from_secs(STALE_LOGIN_STAGING_SECS),
    ) {
        eprintln!("warning: cannot clean stale login temp directories: {e}");
    }
}

fn cleanup_stale_login_staging_in(
    tmp_root: &Path,
    agent_name: &str,
    min_age: Duration,
) -> Result<usize, String> {
    let prefix = format!("{LOGIN_STAGING_PREFIX}{agent_name}-");
    let entries = match fs::read_dir(tmp_root) {
        Ok(entries) => entries,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(0),
        Err(e) => {
            return Err(format!(
                "cannot list temporary directory {}: {e}",
                tmp_root.display()
            ))
        }
    };

    let mut removed = 0;
    let uid = current_uid();

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => {
                eprintln!("warning: cannot read temporary directory entry: {e}");
                continue;
            }
        };

        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }

        let path = entry.path();
        let meta = match fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => {
                return Err(format!("cannot inspect {}: {e}", path.display()));
            }
        };

        if !meta.file_type().is_dir() || meta.uid() != uid {
            continue;
        }
        if login_staging_owner_alive(&path) {
            continue;
        }

        if !min_age.is_zero() {
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let Ok(age) = modified.elapsed() else {
                continue;
            };
            if age < min_age {
                continue;
            }
        }

        fs::remove_dir_all(&path)
            .map_err(|e| format!("cannot remove stale login temp {}: {e}", path.display()))?;
        removed += 1;
    }

    Ok(removed)
}

fn write_login_staging_owner(staging_path: &Path) -> Result<(), String> {
    let owner_file = staging_path.join(LOGIN_STAGING_OWNER_FILE);
    fs::write(&owner_file, process::id().to_string()).map_err(|e| {
        format!(
            "cannot write login staging owner {}: {e}",
            owner_file.display()
        )
    })?;
    fs::set_permissions(&owner_file, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("cannot set permissions on {}: {e}", owner_file.display()))?;
    Ok(())
}

fn login_staging_owner_alive(staging_path: &Path) -> bool {
    let owner_file = staging_path.join(LOGIN_STAGING_OWNER_FILE);
    let Ok(content) = fs::read_to_string(owner_file) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<libc::pid_t>() else {
        return false;
    };
    if pid <= 0 {
        return false;
    }

    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

const AGENT_TOOLS_VOLUME: &str = "agentcage-tools";
const AGENT_TOOLS_DST: &str = "/opt/agent-tools";
const AGENT_TOOLS_INIT_SCRIPT: &str = r#"
set -eu
mkdir -p /opt/agent-tools/bin /opt/agent-tools/lib /opt/agent-tools/lib/node_modules
chmod 0777 /opt/agent-tools /opt/agent-tools/bin /opt/agent-tools/lib /opt/agent-tools/lib/node_modules
"#;

fn agent_tools_mount() -> String {
    format!("type=volume,src={AGENT_TOOLS_VOLUME},dst={AGENT_TOOLS_DST}")
}

fn ensure_tool_cache_volume(podman: &PodmanInfo) -> Result<(), String> {
    let status = Command::new(&podman.executable)
        .arg("run")
        .arg("--rm")
        .arg("--user")
        .arg("0:0")
        .arg("--mount")
        .arg(agent_tools_mount())
        .arg(IMAGE)
        .arg("sh")
        .arg("-lc")
        .arg(AGENT_TOOLS_INIT_SCRIPT)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("podman run failed while preparing tool cache volume: {e}"))?;

    if !status.success() {
        return Err("could not prepare the shared tool cache volume".into());
    }

    Ok(())
}

fn build_run_argv(
    cwd: &Path,
    agent_info: Option<&agent::AgentInfo>,
    extra_args: &[String],
    is_shell: bool,
    home_mount: &HomeMount,
    login_mount: &LoginMount,
) -> Result<(Vec<String>, String), String> {
    let project_src = cwd
        .canonicalize()
        .map_err(|e| format!("cannot resolve project path: {e}"))?;
    let path_str = mount_path_str(&project_src, "project")?;

    let session_id = &nanoid(12);
    let container_name = format!("agentcage-{session_id}");
    let network = container_network();
    let uid = current_uid();
    let gid = current_gid();

    let mut argv = vec![PODMAN.to_string(), "run".to_string(), "--rm".to_string()];

    if is_shell || has_terminal() {
        argv.push("-it".to_string());
    }

    argv.extend([
        "--name".to_string(),
        container_name.clone(),
        "--cap-drop=ALL".to_string(),
        "--security-opt=no-new-privileges".to_string(),
        "--pids-limit=256".to_string(),
        "--memory=4g".to_string(),
        "--cpus=2".to_string(),
        "--http-proxy=false".to_string(),
        "--read-only".to_string(),
        "--read-only-tmpfs=true".to_string(),
        "--userns=keep-id".to_string(),
        "--user".to_string(),
        format!("{uid}:{gid}"),
        "--group-add=keep-groups".to_string(),
        format!("--network={network}"),
        "--mount".to_string(),
        format!("type=bind,src={path_str},dst={CONTAINER_PROJECT},rw=true,relabel=shared"),
        "--mount".to_string(),
        format!(
            "type=tmpfs,destination={CONTAINER_CAGE_DIR},tmpfs-size=64M,tmpfs-mode=0700,U=true"
        ),
        "--workdir".to_string(),
        CONTAINER_PROJECT.to_string(),
    ]);

    match home_mount {
        HomeMount::Disposable => {
            argv.push("--mount".to_string());
            argv.push(format!(
                "type=tmpfs,destination={CONTAINER_HOME},tmpfs-size=512M,tmpfs-mode=0700,U=true"
            ));
        }
        HomeMount::Persistent { host_path, .. } => {
            let host_home = host_path
                .canonicalize()
                .map_err(|e| format!("cannot resolve environment home path: {e}"))?;
            let host_home_str = mount_path_str(&host_home, "environment home")?;
            argv.push("--mount".to_string());
            argv.push(format!(
                "type=bind,src={host_home_str},dst={CONTAINER_HOME},rw=true,relabel=shared"
            ));
        }
    }

    if let LoginMount::Persistent { staging_path, .. } = login_mount {
        let host_login = staging_path
            .canonicalize()
            .map_err(|e| format!("cannot resolve login sync path: {e}"))?;
        let host_login_str = mount_path_str(&host_login, "login sync")?;
        argv.push("--mount".to_string());
        argv.push(format!(
            "type=bind,src={host_login_str},dst={CONTAINER_LOGIN_SYNC},rw=true,relabel=shared"
        ));
    }

    // Named volume for agent tools — persists installed agents across runs
    argv.push("--mount".to_string());
    argv.push(agent_tools_mount());

    for (key, value) in RUNTIME_ENV_VARS {
        argv.push("-e".to_string());
        argv.push(format!("{key}={value}"));
    }

    argv.push(IMAGE.to_string());

    // Build the command to run inside the container.
    // For agent runs, wrap with agentcage-ensure for lazy installation.
    // For shell, just run bash directly.
    let command = if is_shell {
        "bash"
    } else if let Some(info) = agent_info {
        info.command
    } else {
        return Err("no agent or shell specified".into());
    };

    // Build the actual command args (possibly wrapped with install check)
    let ensure_args: Option<Vec<String>> = agent_info.map(|info| {
        let (itype, target) = match info.install {
            agent::AgentInstall::Npm(pkg) => ("npm", pkg.to_string()),
            agent::AgentInstall::Script(cmd) => ("script", cmd.to_string()),
        };
        vec![
            "agentcage-ensure".to_string(),
            info.command.to_string(),
            itype.to_string(),
            target,
        ]
    });

    match login_mount {
        LoginMount::Persistent { paths, .. } => {
            argv.push("bash".to_string());
            argv.push("-c".to_string());
            argv.push(LOGIN_SYNC_SCRIPT.to_string());
            argv.push("agentcage-login-sync".to_string());
            argv.push(CONTAINER_LOGIN_SYNC.to_string());
            argv.push(MAX_LOGIN_FILE_BYTES.to_string());
            argv.push(paths.len().to_string());
            argv.extend(paths.iter().map(|path| path.to_string()));
            if let Some(ensure) = ensure_args {
                argv.extend(ensure);
            }
            argv.push(command.to_string());
            argv.extend(extra_args.iter().cloned());
        }
        LoginMount::None | LoginMount::Disabled => {
            if let Some(ensure) = ensure_args {
                argv.extend(ensure);
            }
            argv.push(command.to_string());
            argv.extend(extra_args.iter().cloned());
        }
    }

    Ok((argv, container_name))
}

fn mount_path_str<'a>(path: &'a Path, label: &str) -> Result<&'a str, String> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("{label} path is not valid UTF-8"))?;
    if value.contains(',') {
        return Err(format!(
            "{label} path contains ',' which cannot be safely encoded for Podman --mount: {}",
            path.display()
        ));
    }
    Ok(value)
}

fn container_network() -> &'static str {
    // Codex and similar CLIs run a short-lived OAuth callback server on
    // 127.0.0.1 inside the container. A published bridge port targets the
    // container's non-loopback address, so browser redirects to
    // http://localhost:1455/... can fail even though -p was configured.
    // Sharing the host network keeps localhost identical for the CLI and
    // the user's browser.
    "host"
}

fn run_podman(
    argv: &[String],
    container_name: &str,
    login_mount: &LoginMount,
) -> Result<(), String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("empty command".into());
    };

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            cleanup_login_staging(login_mount);
            format!("cannot start podman: {e}")
        })?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_thread = thread::spawn(move || forward_stream(stdout, io::stdout()));
    let stderr_thread = thread::spawn(move || forward_stream(stderr, io::stderr()));

    let status = match child.wait() {
        Ok(status) => status,
        Err(e) => {
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            cleanup_login_staging(login_mount);
            return Err(format!("podman exited with error: {e}"));
        }
    };

    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    finalize_login_mount(login_mount)?;

    if !status.success() {
        let code = status.code().unwrap_or(1);
        if code == 130 {
            return Ok(());
        }
        let _ = Command::new(PODMAN)
            .args(["rm", "-f", container_name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        return Err(format!("container command exited with status {code}"));
    }

    Ok(())
}

fn forward_stream<R: Read, W: io::Write>(mut reader: R, mut writer: W) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if writer.write_all(&buf[..n]).is_err() {
                    break;
                }
                let _ = writer.flush();
            }
            Err(_) => break,
        }
    }
}

struct PodmanInfo {
    executable: String,
    version: String,
    rootless: bool,
    image_exists: bool,
    image_ready: bool,
}

fn detect_podman() -> Result<PodmanInfo, String> {
    let executable = find_podman()?;
    let version = podman_version(&executable)?;
    let rootless = podman_rootless(&executable)?;
    if !rootless {
        return Err("Rootless Podman is required. Run `podman info --format '{{.Host.Security.Rootless}}'` and configure Podman rootless mode.".into());
    }
    let image_exists = podman_image_exists(&executable)?;
    let image_ready = if image_exists {
        podman_image_user_matches(&executable)? && podman_image_hash_matches(&executable)?
    } else {
        false
    };
    Ok(PodmanInfo {
        executable,
        version,
        rootless,
        image_exists,
        image_ready,
    })
}

fn ensure_runtime_image(podman: &PodmanInfo) -> Result<(), String> {
    if podman.image_ready {
        return Ok(());
    }

    if !podman.image_exists {
        println!("AgentCage: local runtime image not found.");
    } else {
        println!("AgentCage: local runtime image is outdated and will be rebuilt.");
    }

    let status = build_image(podman)?;
    if !status.success() {
        return Err("image build failed. check the output above".into());
    }
    if !podman_image_exists(&podman.executable)? {
        return Err(format!("image '{IMAGE}' still not found after rebuild"));
    }
    if !podman_image_user_matches(&podman.executable)?
        || !podman_image_hash_matches(&podman.executable)?
    {
        return Err(format!(
            "image '{IMAGE}' was rebuilt but is still not ready for this host user"
        ));
    }
    println!("AgentCage: runtime image ready.");
    println!();
    Ok(())
}

fn find_podman() -> Result<String, String> {
    match Command::new(PODMAN).arg("--version").output() {
        Ok(_) => Ok(PODMAN.into()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            if Command::new("docker").arg("--version").output().is_ok() {
                Err(format!(
                    "Docker detected but Podman is required. {}",
                    podman_install_hint()
                ))
            } else {
                Err(format!("Podman not found. {}", podman_install_hint()))
            }
        }
        Err(e) => Err(format!("cannot check for Podman: {e}")),
    }
}

fn podman_version(executable: &str) -> Result<String, String> {
    let output = Command::new(executable)
        .arg("version")
        .arg("--format")
        .arg("{{.Version}}")
        .output()
        .map_err(|e| format!("podman version failed: {e}"))?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        return Err("could not determine Podman version".into());
    }
    Ok(version)
}

fn podman_rootless(executable: &str) -> Result<bool, String> {
    let output = Command::new(executable)
        .args(["info", "--format", "{{.Host.Security.Rootless}}"])
        .output()
        .map_err(|e| format!("podman info failed: {e}"))?;
    match String::from_utf8_lossy(&output.stdout).trim() {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("unexpected podman rootless value: '{other}'")),
    }
}

fn podman_image_exists(executable: &str) -> Result<bool, String> {
    let status = Command::new(executable)
        .args(["image", "exists", IMAGE])
        .status()
        .map_err(|e| format!("podman image exists failed: {e}"))?;
    Ok(status.success())
}

fn podman_image_user_matches(executable: &str) -> Result<bool, String> {
    let output = Command::new(executable)
        .args(["image", "inspect", IMAGE, "--format", "{{.Config.User}}"])
        .output()
        .map_err(|e| format!("podman image inspect failed: {e}"))?;
    if !output.status.success() {
        return Err("podman image inspect failed".into());
    }
    let image_user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(image_user == format!("{}:{}", current_uid(), current_gid()))
}

fn get_dockerfile_hash() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut s = DefaultHasher::new();
    IMAGE_DOCKERFILE.hash(&mut s);
    format!("{:x}", s.finish())
}

fn podman_image_hash_matches(executable: &str) -> Result<bool, String> {
    let output = Command::new(executable)
        .args([
            "image",
            "inspect",
            IMAGE,
            "--format",
            "{{index .Config.Labels \"agentcage.dockerfile.hash\"}}",
        ])
        .output()
        .map_err(|e| format!("podman image inspect failed: {e}"))?;
    if !output.status.success() {
        return Err("podman image inspect failed".into());
    }
    let image_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(image_hash == get_dockerfile_hash())
}

fn tool_cache_status(podman: &PodmanInfo, agent_info: &agent::AgentInfo) -> Result<bool, String> {
    let check = format!("command -v {} >/dev/null 2>&1", agent_info.command);
    let status = Command::new(&podman.executable)
        .arg("run")
        .arg("--rm")
        .arg("--mount")
        .arg(agent_tools_mount())
        .arg(IMAGE)
        .arg("sh")
        .arg("-lc")
        .arg(check)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("podman run failed: {e}"))?;
    Ok(status.success())
}

fn build_image(podman: &PodmanInfo) -> Result<process::ExitStatus, String> {
    let context_dir = create_private_temp_dir("agentcage-build-")?;

    let dockerfile = context_dir.join("Dockerfile");
    fs::write(&dockerfile, IMAGE_DOCKERFILE)
        .map_err(|e| format!("cannot write temporary Dockerfile: {e}"))?;

    println!("Step 1/2: building local runtime image {IMAGE}...");
    println!("         This image contains the shared agent bootstrap tools.");
    let status = Command::new(&podman.executable)
        .arg("build")
        .arg("-t")
        .arg(IMAGE)
        .arg("--label")
        .arg(format!(
            "agentcage.dockerfile.hash={}",
            get_dockerfile_hash()
        ))
        .arg("--build-arg")
        .arg(format!("UID={}", current_uid()))
        .arg("--build-arg")
        .arg(format!("GID={}", current_gid()))
        .arg("-f")
        .arg(&dockerfile)
        .arg(&context_dir)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| format!("podman build failed: {e}"));

    if let Err(e) = fs::remove_dir_all(&context_dir) {
        eprintln!(
            "warning: cannot remove temporary image build directory {}: {e}",
            context_dir.display()
        );
    }

    status
}

fn podman_install_hint() -> String {
    let info = linux_distribution_info();
    match info.id.as_deref() {
        Some("fedora") => {
            "AgentCage was developed on Fedora, where Podman is usually already available. If it is missing, run: sudo dnf install -y podman".into()
        }
        Some("ubuntu") | Some("debian") => {
            "Install rootless Podman first, for example: sudo apt install -y podman".into()
        }
        Some("arch") => "Install rootless Podman first, for example: sudo pacman -S podman".into(),
        Some("opensuse-tumbleweed") | Some("opensuse-leap") => {
            "Install rootless Podman first, for example: sudo zypper install -y podman".into()
        }
        _ => {
            if let Some(name) = info.name {
                format!(
                    "Install rootless Podman on {} and verify `podman info --format '{{{{.Host.Security.Rootless}}}}'` prints `true`. See https://podman.io",
                    name
                )
            } else {
                "Install rootless Podman and verify `podman info --format '{{.Host.Security.Rootless}}'` prints `true`. See https://podman.io".into()
            }
        }
    }
}

struct LinuxDistributionInfo {
    id: Option<String>,
    name: Option<String>,
}

fn linux_distribution_info() -> LinuxDistributionInfo {
    let content = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let mut id = None;
    let mut name = None;

    for line in content.lines() {
        if let Some(value) = line.strip_prefix("ID=") {
            id = Some(trim_os_release_value(value));
        } else if let Some(value) = line.strip_prefix("NAME=") {
            name = Some(trim_os_release_value(value));
        }
    }

    LinuxDistributionInfo { id, name }
}

fn trim_os_release_value(value: &str) -> String {
    value.trim_matches('"').to_string()
}

fn has_terminal() -> bool {
    unsafe { libc::isatty(0) != 0 }
}

fn current_uid() -> u32 {
    unsafe { libc::geteuid() }
}

fn current_gid() -> u32 {
    unsafe { libc::getegid() }
}

fn nanoid(len: usize) -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut buf = String::with_capacity(len);
    let mut seed: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    for _ in 0..len {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let idx = (seed >> 33) as usize % ALPHABET.len();
        buf.push(ALPHABET[idx] as char);
    }
    buf
}

fn print_help() {
    print!(
        "AgentCage — run AI coding agents in a disposable container

Only the current repo is mounted. The host home directory is not mounted.
The container is deleted on exit. Home is ephemeral by default.
Agent login credentials are saved separately, with only auth files persisted.
Use --env NAME to keep a full persistent container home for settings too.

Requirements:
  Linux with rootless Podman (Docker is not supported).
  Fedora is the primary tested target; other Linux distros need Podman installed/configured first.
  Rootless check: podman info --format '{{{{.Host.Security.Rootless}}}}' -> true

Usage:
  ac
  ac <command> [options]

Commands:
  init          same as ac: check runtime, build image, set up project
  codex         run Codex CLI in a disposable cage
  claude        run Claude Code in a disposable cage
  opencode      run Opencode in a disposable cage
  antigravity   run Google Antigravity CLI in a disposable cage
  shell         open a shell inside the cage
  env           list/remove persistent environments
  login         list/remove saved login credentials
  clean         remove leftover containers
  help          show this help

Options:
  --env NAME    use full persistent environment home (.agentcage/envs/NAME/home)
  --no-login    do not sync saved agent login into this run (agent commands only)

Examples:
  cd my-project
  ac
  ac opencode   # first run may install opencode into the shared tool cache
  ac codex
  ac codex --no-login
  ac antigravity
  ac codex --env my-oauth
  ac env list
  ac env rm my-oauth
  ac login list
  ac login rm antigravity
  ac shell

Tab completion:
  source <(ac completions bash)
  source <(ac completions zsh)
  ac completions fish | source
"
    );
}

fn print_completions(shell: &str) {
    match shell {
        "bash" => print_completions_bash(),
        "zsh" => print_completions_zsh(),
        "fish" => print_completions_fish(),
        _ => {
            eprintln!("usage: {CLI} completions <bash|zsh|fish>");
            process::exit(1);
        }
    }
}

fn print_completions_bash() {
    print!(
        r#"_ac_completions() {{
    local cur prev words cword
    _init_completion -s || return
    COMPREPLY=()

    local cmd="${{words[1]}}"
    case "$cword" in
        1)
            COMPREPLY=($(compgen -W "init codex claude opencode antigravity shell env login remove clean help completions" -- "$cur"))
            ;;
        2)
            case "$cmd" in
                env)
                    COMPREPLY=($(compgen -W "list rm remove delete" -- "$cur"))
                    ;;
                login)
                    COMPREPLY=($(compgen -W "list rm remove delete" -- "$cur"))
                    ;;
                remove)
                    COMPREPLY=($(compgen -W "env login" -- "$cur"))
                    ;;
                codex|claude|opencode|antigravity)
                    COMPREPLY=($(compgen -W "--env --no-login --help" -- "$cur"))
                    ;;
                shell)
                    COMPREPLY=($(compgen -W "--env --help" -- "$cur"))
                    ;;
            esac
            ;;
        *)
            case "$cmd" in
                env)
                    if [[ "${{words[2]}}" == "rm" || "${{words[2]}}" == "remove" || "${{words[2]}}" == "delete" ]]; then
                        if [[ -d .agentcage/envs ]]; then
                            COMPREPLY=($(compgen -W "$(command ls -1 .agentcage/envs 2>/dev/null)" -- "$cur"))
                        fi
                    fi
                    ;;
                login)
                    if [[ "${{words[2]}}" == "rm" || "${{words[2]}}" == "remove" || "${{words[2]}}" == "delete" ]]; then
                        COMPREPLY=($(compgen -W "codex claude opencode antigravity" -- "$cur"))
                    fi
                    ;;
                remove)
                    if [[ "${{words[2]}}" == "env" ]]; then
                        if [[ -d .agentcage/envs ]]; then
                            COMPREPLY=($(compgen -W "$(command ls -1 .agentcage/envs 2>/dev/null)" -- "$cur"))
                        fi
                    elif [[ "${{words[2]}}" == "login" ]]; then
                        COMPREPLY=($(compgen -W "codex claude opencode antigravity" -- "$cur"))
                    fi
                    ;;
            esac
            ;;
    esac
}} && complete -F _ac_completions ac
"#
    );
}

fn print_completions_zsh() {
    print!(
        r#"#compdef ac

_ac() {{
    _arguments -C \
        '1: :_ac_cmds' \
        '2: :_ac_subcmds' \
        '*:: :->args'

    case "$state" in
        args)
            local cmd="$words[1]"
            case "$cmd" in
                (codex|claude|opencode|antigravity)
                    _arguments \
                        '--env[persistent environment name]:environment name:' \
                        '--no-login[do not sync saved login into this run]'
                    ;;
                (shell)
                    _arguments \
                        '--env[persistent environment name]:environment name:'
                    ;;
                (env)
                    case "$words[2]" in
                        (rm|remove|delete)
                            _arguments '3:environment name:'
                            ;;
                    esac
                    ;;
                (login)
                    case "$words[2]" in
                        (rm|remove|delete)
                            _arguments '3:agent:(codex claude opencode antigravity)'
                            ;;
                    esac
                    ;;
                (remove)
                    case "$words[2]" in
                        (env)
                            _arguments '3:environment name:'
                            ;;
                        (login)
                            _arguments '3:agent:(codex claude opencode antigravity)'
                            ;;
                    esac
                    ;;
            esac
            ;;
    esac
}}

_ac_cmds() {{
    _values 'command' \
        'init[check runtime and build image]' \
        'codex[run Codex CLI]' \
        'claude[run Claude Code]' \
        'opencode[run Opencode]' \
        'antigravity[run Google Antigravity CLI]' \
        'shell[open a shell in the cage]' \
        'env[list/remove persistent environments]' \
        'login[list/remove saved logins]' \
        'remove[alias command for removals]' \
        'clean[remove leftover containers]' \
        'help[show help]' \
        'completions[generate shell completions]'
}}

_ac_subcmds() {{
    local cmd="$words[1]"
    case "$cmd" in
        (env)
            _values 'env command' \
                'list[list persistent environments]' \
                'rm[remove persistent environment]' \
                'remove[remove persistent environment]' \
                'delete[remove persistent environment]'
            ;;
        (login)
            _values 'login command' \
                'list[list saved logins]' \
                'rm[remove saved login]' \
                'remove[remove saved login]' \
                'delete[remove saved login]'
            ;;
        (remove)
            _values 'remove target' \
                'env[remove persistent environment]' \
                'login[remove saved login]'
            ;;
        (completions)
            _values 'shell' bash zsh fish
            ;;
    esac
}}

_ac
"#
    );
}

fn print_completions_fish() {
    print!(
        r#"complete -c ac -f

# subcommands
complete -c ac -n "__fish_use_subcommand" -a init -d "check runtime and build image"
complete -c ac -n "__fish_use_subcommand" -a codex -d "run Codex CLI"
complete -c ac -n "__fish_use_subcommand" -a claude -d "run Claude Code"
complete -c ac -n "__fish_use_subcommand" -a opencode -d "run Opencode"
complete -c ac -n "__fish_use_subcommand" -a antigravity -d "run Google Antigravity CLI"
complete -c ac -n "__fish_use_subcommand" -a shell -d "open a shell in the cage"
complete -c ac -n "__fish_use_subcommand" -a env -d "list/remove persistent environments"
complete -c ac -n "__fish_use_subcommand" -a login -d "list/remove saved logins"
complete -c ac -n "__fish_use_subcommand" -a remove -d "alias command for removals"
complete -c ac -n "__fish_use_subcommand" -a clean -d "remove leftover containers"
complete -c ac -n "__fish_use_subcommand" -a help -d "show help"
complete -c ac -n "__fish_use_subcommand" -a completions -d "generate shell completions"

# env subcommands
complete -c ac -n "__fish_seen_subcommand_from env; and not __fish_seen_subcommand_from list rm remove delete" -a list -d "list persistent environments"
complete -c ac -n "__fish_seen_subcommand_from env; and not __fish_seen_subcommand_from list rm remove delete" -a rm -d "remove persistent environment"
complete -c ac -n "__fish_seen_subcommand_from env; and not __fish_seen_subcommand_from list rm remove delete" -a remove -d "remove persistent environment"
complete -c ac -n "__fish_seen_subcommand_from env; and not __fish_seen_subcommand_from list rm remove delete" -a delete -d "remove persistent environment"

# login subcommands
complete -c ac -n "__fish_seen_subcommand_from login; and not __fish_seen_subcommand_from list rm remove delete" -a list -d "list saved logins"
complete -c ac -n "__fish_seen_subcommand_from login; and not __fish_seen_subcommand_from list rm remove delete" -a rm -d "remove saved login"
complete -c ac -n "__fish_seen_subcommand_from login; and not __fish_seen_subcommand_from list rm remove delete" -a remove -d "remove saved login"
complete -c ac -n "__fish_seen_subcommand_from login; and not __fish_seen_subcommand_from list rm remove delete" -a delete -d "remove saved login"
complete -c ac -n "__fish_seen_subcommand_from login; and __fish_seen_subcommand_from rm remove delete" -a "codex claude opencode antigravity" -d "saved login"

# remove targets
complete -c ac -n "__fish_seen_subcommand_from remove; and not __fish_seen_subcommand_from env login" -a env -d "remove persistent environment"
complete -c ac -n "__fish_seen_subcommand_from remove; and not __fish_seen_subcommand_from env login" -a login -d "remove saved login"
complete -c ac -n "__fish_seen_subcommand_from remove; and __fish_seen_subcommand_from login" -a "codex claude opencode antigravity" -d "saved login"

# options for agent/shell subcommands
complete -c ac -n "__fish_seen_subcommand_from codex claude opencode antigravity shell" -l env -r -d "persistent environment name"
complete -c ac -n "__fish_seen_subcommand_from codex claude opencode antigravity" -l no-login -d "do not sync saved login into this run"
"#
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_test_root(name: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "agentcage-test-{}-{}-{}",
            name,
            process::id(),
            nanoid(8)
        ))
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn parse_err(args: &[&str]) -> String {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        match parse(&args) {
            Ok(_) => panic!("expected parse error"),
            Err(e) => e,
        }
    }

    #[test]
    fn runs_use_host_network_for_localhost_oauth_callbacks() {
        let cwd = env::current_dir().unwrap();
        let codex_info = agent::resolve("codex").unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            Some(codex_info),
            &[],
            false,
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(argv.iter().any(|arg| arg == "--network=host"));
        assert!(!argv.iter().any(|arg| arg == "-p"));
        assert!(!argv.iter().any(|arg| arg.contains("1455")));
    }

    #[test]
    fn runs_hide_project_agentcage_state_with_tmpfs() {
        let cwd = env::current_dir().unwrap();
        let codex_info = agent::resolve("codex").unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            Some(codex_info),
            &[],
            false,
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(argv
            .iter()
            .any(|arg| arg.contains(&format!("type=tmpfs,destination={CONTAINER_CAGE_DIR}"))));
    }

    #[test]
    fn no_login_runs_without_login_sync_mount_or_wrapper() {
        let cwd = env::current_dir().unwrap();
        let codex_info = agent::resolve("codex").unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            Some(codex_info),
            &[],
            false,
            &HomeMount::Disposable,
            &LoginMount::Disabled,
        )
        .unwrap();

        assert!(!argv.iter().any(|arg| arg.contains(CONTAINER_LOGIN_SYNC)));
        assert!(!argv.iter().any(|arg| arg == "agentcage-login-sync"));
        assert!(argv.iter().any(|arg| arg == "codex"));
    }

    #[test]
    fn rejects_unknown_global_options() {
        let err = parse_err(&["--bogus"]);
        assert!(err.contains("unknown global option"));
    }

    #[test]
    fn completions_requires_supported_shell() {
        let missing = parse_err(&["completions"]);
        assert!(missing.contains("usage: ac completions"));

        let invalid = parse_err(&["completions", "powershell"]);
        assert!(invalid.contains("usage: ac completions"));
    }

    #[test]
    fn parses_no_login_for_agent_commands() {
        let cmd = parse(&[
            "codex".to_string(),
            "--no-login".to_string(),
            "--".to_string(),
            "--help".to_string(),
        ])
        .unwrap();

        match cmd {
            Cmd::Agent { login, args, .. } => {
                assert_eq!(login, LoginPolicy::Disabled);
                assert_eq!(args, vec!["--help".to_string()]);
            }
            _ => panic!("expected agent command"),
        }
    }

    #[test]
    fn ensure_cage_dir_rejects_symlink() {
        let root = temp_test_root("cage-symlink");
        fs::create_dir_all(&root).unwrap();
        let target = temp_test_root("cage-symlink-target");
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, root.join(CAGE_DIR_NAME)).unwrap();

        let err = ensure_cage_dir(&root).unwrap_err();
        assert!(err.contains("symlinked project state directory"));

        let _ = fs::remove_dir_all(root);
        let _ = fs::remove_dir_all(target);
    }

    #[test]
    fn stale_login_staging_cleanup_removes_only_matching_agent_dirs() {
        let root = temp_test_root("stale-login");
        fs::create_dir_all(&root).unwrap();
        let matching = root.join(format!("{LOGIN_STAGING_PREFIX}codex-deadbeef"));
        let other_agent = root.join(format!("{LOGIN_STAGING_PREFIX}claude-deadbeef"));
        let file = root.join(format!("{LOGIN_STAGING_PREFIX}codex-file"));
        fs::create_dir_all(&matching).unwrap();
        fs::create_dir_all(&other_agent).unwrap();
        fs::write(&file, "not a dir").unwrap();

        let removed =
            cleanup_stale_login_staging_in(&root, "codex", Duration::from_secs(0)).unwrap();

        assert_eq!(removed, 1);
        assert!(!matching.exists());
        assert!(other_agent.exists());
        assert!(file.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_login_staging_cleanup_keeps_fresh_dirs() {
        let root = temp_test_root("fresh-login");
        fs::create_dir_all(&root).unwrap();
        let matching = root.join(format!("{LOGIN_STAGING_PREFIX}codex-fresh"));
        fs::create_dir_all(&matching).unwrap();

        let removed =
            cleanup_stale_login_staging_in(&root, "codex", Duration::from_secs(60)).unwrap();

        assert_eq!(removed, 0);
        assert!(matching.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn stale_login_staging_cleanup_keeps_live_owner_dirs() {
        let root = temp_test_root("live-login");
        fs::create_dir_all(&root).unwrap();
        let matching = root.join(format!("{LOGIN_STAGING_PREFIX}codex-live"));
        fs::create_dir_all(&matching).unwrap();
        write_login_staging_owner(&matching).unwrap();

        let removed =
            cleanup_stale_login_staging_in(&root, "codex", Duration::from_secs(0)).unwrap();

        assert_eq!(removed, 0);
        assert!(matching.exists());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn antigravity_maps_to_agy_and_persists_only_token_file() {
        assert_eq!(agent::resolve("antigravity").unwrap().command, "agy");
        assert_eq!(
            login_paths_for_agent("antigravity"),
            &[".gemini/antigravity-cli/antigravity-oauth-token"]
        );
    }

    #[test]
    fn opencode_persists_auth_and_account_files() {
        assert_eq!(
            login_paths_for_agent("opencode"),
            &[
                ".local/share/opencode/auth.json",
                ".local/share/opencode/account.json",
            ]
        );
    }

    #[test]
    fn agent_run_includes_tools_volume() {
        let cwd = env::current_dir().unwrap();
        let codex_info = agent::resolve("codex").unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            Some(codex_info),
            &[],
            false,
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(argv.iter().any(|arg| arg.contains(AGENT_TOOLS_VOLUME)));
        assert!(argv.iter().any(|arg| arg == "agentcage-ensure"));
    }

    #[test]
    fn shell_run_has_no_install_wrapper() {
        let cwd = env::current_dir().unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            None,
            &[],
            true,
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(!argv.iter().any(|arg| arg == "agentcage-ensure"));
        assert!(argv.iter().any(|arg| arg == "bash"));
    }

    #[test]
    fn persistent_env_dirs_are_private() {
        let root = temp_test_root("private-env");
        let home = create_private_subdir_all(&root, Path::new("envs/oauth/home")).unwrap();

        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("envs")), 0o700);
        assert_eq!(mode(&root.join("envs/oauth")), 0o700);
        assert_eq!(mode(&home), 0o700);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn known_login_files_in_persistent_home_are_private() {
        let home = temp_test_root("home-logins");
        let auth_dir = home.join(".local/share/opencode");
        fs::create_dir_all(&auth_dir).unwrap();
        let auth_file = auth_dir.join("auth.json");
        fs::write(&auth_file, "{}").unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&auth_dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&auth_file, fs::Permissions::from_mode(0o644)).unwrap();

        harden_known_login_files_in_home(&home).unwrap();

        assert_eq!(mode(&home), 0o700);
        assert_eq!(mode(&home.join(".local")), 0o700);
        assert_eq!(mode(&home.join(".local/share")), 0o700);
        assert_eq!(mode(&auth_dir), 0o700);
        assert_eq!(mode(&auth_file), 0o600);

        let _ = fs::remove_dir_all(home);
    }
}

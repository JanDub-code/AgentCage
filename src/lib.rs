use std::cell::Cell;
use std::env;
use std::ffi::{CString, OsString};
use std::fs;
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::process::ExitStatusExt;
use std::path::{Component, Path, PathBuf};
use std::process::{self, Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
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
const LOGIN_STAGING_COMPLETE_FILE: &str = ".agentcage-copy-complete";
const STALE_LOGIN_STAGING_SECS: u64 = 24 * 60 * 60;
const MAX_LOGIN_FILE_BYTES: u64 = 10 * 1024 * 1024;
const CONTAINER_OWNER_LABEL: &str = "io.agentcage.managed=true";
const OPENCODE_ACCOUNT_BRIDGE: &str = include_str!("opencode_account.py");
const LOGIN_FILE_SYNC: &str = include_str!("login_sync.py");
// OCI process arguments are JSON strings. Transport opaque Linux argv as
// bounded ASCII chunks, then exec the agent with the original bytes.
const OPAQUE_ARGV_EXEC: &str = r#"import os, sys
argv = [os.fsencode(sys.argv[1])]
position = 2
while position < len(sys.argv):
    count = int(sys.argv[position])
    position += 1
    argv.append(bytes.fromhex(''.join(sys.argv[position:position + count])))
    position += count
os.execvp(argv[0], argv)
"#;

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
account_bridge="$1"
shift
file_sync="$1"
shift

python3 -c "$file_sync" "$store" "$HOME" "$max_bytes" "${paths[@]}" || exit 1
if [ -n "$account_bridge" ]; then
    opencode db 'SELECT 1' > /dev/null || exit 1
    python3 -c "$account_bridge" import || exit 1
fi
interrupted=0
trap 'interrupted=130' INT
trap 'interrupted=143' TERM
trap 'interrupted=129' HUP
"$@"
status=$?
if [ "$interrupted" -ne 0 ]; then status="$interrupted"; fi
if [ -n "$account_bridge" ] && ! python3 -c "$account_bridge" export; then
    # A failed account export leaves an unvalidated session snapshot. Keep the
    # original staging files instead of publishing any partial login changes.
    if [ "$status" -eq 0 ]; then status=1; fi
    exit "$status"
fi
if ! python3 -c "$file_sync" --export "$HOME" "$store" "$max_bytes" "${paths[@]}"; then
    echo "agentcage: failed to save login credentials" >&2
    if [ "$status" -eq 0 ]; then status=1; fi
fi
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
    (
        "OPENCODE_DB",
        "/workspace/session-home/.local/share/opencode/opencode.db",
    ),
];

pub fn run() {
    // A closed output pipe is ordinary Unix CLI termination, not a Rust panic.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
    let args: Vec<OsString> = env::args_os().skip(1).collect();
    match dispatch(&args) {
        Ok(code) => process::exit(code),
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
        args: Vec<OsString>,
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
        staging_dir: fs::File,
        paths: &'static [&'static str],
        baseline: Vec<Option<Vec<u8>>>,
        revocation: u64,
        retain_on_save_failure: Cell<bool>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LoginPolicy {
    Auto,
    Disabled,
}

fn dispatch(args: &[OsString]) -> Result<i32, String> {
    let cmd = parse_os(args)?;
    let cwd = || env::current_dir().map_err(|e| format!("cannot read current directory: {e}"));
    match cmd {
        Cmd::Init => cmd_init(&cwd()?).map(|_| 0),
        Cmd::Agent {
            name,
            env,
            login,
            args: extra,
        } => cmd_agent(&cwd()?, &name, env.as_deref(), login, &extra),
        Cmd::Shell { env } => cmd_shell(&cwd()?, env.as_deref()),
        Cmd::Clean => cmd_clean().map(|_| 0),
        Cmd::EnvList => cmd_env_list(&cwd()?).map(|_| 0),
        Cmd::EnvRemove { name } => cmd_env_remove(&cwd()?, &name).map(|_| 0),
        Cmd::LoginList => cmd_login_list().map(|_| 0),
        Cmd::LoginRemove { agent } => cmd_login_remove(&agent).map(|_| 0),
        Cmd::Help => {
            print_help();
            Ok(0)
        }
        Cmd::Completions { shell } => {
            print_completions(&shell);
            Ok(0)
        }
    }
}

fn parse_os(args: &[OsString]) -> Result<Cmd, String> {
    let agent_command = args
        .first()
        .and_then(|arg| arg.to_str())
        .map(|name| agent::resolve(name).is_ok())
        .unwrap_or(false);
    let mut wrapper = Vec::new();
    let mut env_value = false;
    for (index, arg) in args.iter().enumerate() {
        if index > 0 && agent_command && !env_value && arg == "--" {
            wrapper.push("--".to_string());
            let mut parsed = parse(&wrapper)?;
            if let Cmd::Agent { args: extra, .. } = &mut parsed {
                // Unix agent arguments are opaque bytes, including filenames
                // that cannot be represented in UTF-8.
                extra.extend_from_slice(&args[index + 1..]);
            }
            return Ok(parsed);
        }
        let text = arg
            .to_str()
            .ok_or("wrapper arguments must be UTF-8; pass opaque agent arguments after '--'")?;
        wrapper.push(text.to_string());
        if index > 0 && agent_command {
            if env_value {
                env_value = false;
            } else if text == "--env" {
                env_value = true;
            }
        }
    }
    parse(&wrapper)
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
                        extra.extend_from_slice(&rest[i + 1..]);
                        break;
                    }
                    arg if arg.starts_with('-') => return Err(format!("unknown wrapper option '{arg}'. pass agent options after '--', for example: {CLI} {name} -- {arg}")),
                    arg => extra.push(arg.to_string()),
                }
                i += 1;
            }
            if matches!(login, LoginPolicy::Disabled) && env.is_some() {
                return Err("--no-login cannot be combined with --env because --env reuses a full persistent home that may already contain credentials".into());
            }
            Ok(Cmd::Agent {
                name,
                env,
                login,
                args: extra.into_iter().map(OsString::from).collect(),
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

    if !podman.image_ready {
        let status = build_image(&podman)?;
        if !status.success() {
            return Err("image build failed. check the output above".into());
        }
    }

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
    extra_args: &[OsString],
) -> Result<i32, String> {
    // The forwarding threads must report a closed pipe and still finalize
    // credentials and reap Podman, rather than terminate the parent early.
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    ensure_cage_dir(cwd)?;
    if matches!(login_policy, LoginPolicy::Disabled) && env_name.is_some() {
        return Err("--no-login cannot be combined with --env because --env reuses a full persistent home that may already contain credentials".into());
    }

    let command = agent::resolve(agent_name).map_err(|e| e)?;
    let podman = detect_podman()?;
    if !podman.image_exists {
        return Err(format!("image '{IMAGE}' not found. run '{CLI}' first"));
    }
    if !podman.image_ready {
        return Err(format!(
            "image '{IMAGE}' is outdated or was built for a different host UID/GID. run '{CLI}' to rebuild it"
        ));
    }

    let home_mount = resolve_home_mount(cwd, env_name)?;
    let login_mount = resolve_login_mount(agent_name, &home_mount, login_policy)?;
    print_status(cwd, &home_mount, &login_mount);
    let result = (|| {
        let (argv, container_name) =
            build_run_argv(cwd, command, extra_args, &home_mount, &login_mount)?;
        run_podman(&argv, &container_name, &login_mount)
    })();
    let result = combine_session_result(result, cleanup_login_staging(&login_mount));
    combine_session_result(result, harden_persistent_home(&home_mount))
}

fn cmd_shell(cwd: &Path, env_name: Option<&str>) -> Result<i32, String> {
    unsafe { libc::signal(libc::SIGPIPE, libc::SIG_IGN) };
    ensure_cage_dir(cwd)?;
    let podman = detect_podman()?;
    if !podman.image_exists {
        return Err(format!("image '{IMAGE}' not found. run '{CLI}' first"));
    }
    if !podman.image_ready {
        return Err(format!(
            "image '{IMAGE}' is outdated or was built for a different host UID/GID. run '{CLI}' to rebuild it"
        ));
    }
    let home_mount = resolve_home_mount(cwd, env_name)?;
    let login_mount = LoginMount::None;
    print_status(cwd, &home_mount, &login_mount);
    let (argv, container_name) = build_run_argv(cwd, "bash", &[], &home_mount, &login_mount)?;
    let result = run_podman(&argv, &container_name, &login_mount);
    combine_session_result(result, harden_persistent_home(&home_mount))
}

fn combine_session_result(
    result: Result<i32, String>,
    cleanup: Result<(), String>,
) -> Result<i32, String> {
    match (result, cleanup) {
        (Ok(0), Err(e)) => Err(e),
        (Ok(code), Err(e)) => {
            eprintln!("error: {e}");
            Ok(code)
        }
        (Err(e), Err(cleanup)) => Err(format!("{e}; cleanup also failed: {cleanup}")),
        (result, Ok(())) => result,
    }
}

fn cmd_clean() -> Result<(), String> {
    let podman = detect_podman()?;
    let output = Command::new(&podman.executable)
        .args([
            "ps",
            "-a",
            "--filter",
            "label=io.agentcage.managed=true",
            "--filter",
            "status=exited",
            "--filter",
            "status=created",
            "--filter",
            "status=stopped",
            "--no-trunc",
            "--format",
            "{{.ID}}",
        ])
        .output()
        .map_err(|e| format!("podman ps failed: {e}"))?;
    check_podman_output("podman ps", &output)?;
    let ids = String::from_utf8_lossy(&output.stdout);
    let containers: Vec<&str> = ids.lines().filter(|l| !l.is_empty()).collect();
    if containers.is_empty() {
        println!("AgentCage — nothing to clean");
        return Ok(());
    }
    let mut failures = Vec::new();
    for id in containers {
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err("podman ps returned an invalid container ID; cleanup stopped".into());
        }
        let result = Command::new(&podman.executable)
            .args(["rm", "--ignore", id])
            .output()
            .map_err(|e| format!("podman rm failed: {e}"))
            .and_then(|output| {
                check_podman_output("podman rm", &output)?;
                if output.stdout.is_empty() {
                    println!("container already absent: {id}");
                } else {
                    println!("removed container: {id}");
                }
                Ok(())
            });
        if let Err(e) = result {
            eprintln!("cannot remove {id}: {e}");
            failures.push(id);
        }
    }
    if failures.is_empty() {
        println!("cleanup complete");
        Ok(())
    } else {
        Err(format!(
            "cleanup incomplete: {} container(s) could not be removed",
            failures.len()
        ))
    }
}

fn cmd_env_list(cwd: &Path) -> Result<(), String> {
    let cage = open_private_root(&ensure_cage_dir(cwd)?)?;
    let Some((envs, _)) = relative_parent(&cage, "envs/.unused", false)? else {
        println!("no persistent environments");
        return Ok(());
    };
    let entries = fs::read_dir(format!("/proc/self/fd/{}", envs.as_raw_fd()))
        .map_err(|e| format!("cannot list persistent environments: {e}"))?;

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
    let cage = open_private_root(&ensure_cage_dir(cwd)?)?;
    let Some((_parent, anchored_path)) = directory_for_removal(&cage, &format!("envs/{name}"))?
    else {
        return Err(format!("environment '{name}' does not exist"));
    };
    match fs::remove_dir_all(&anchored_path) {
        Ok(()) => {
            println!("removed environment: {name}");
            Ok(())
        }
        Err(e) => Err(format!("cannot remove environment '{}': {e}", name)),
    }
}

fn cmd_login_list() -> Result<(), String> {
    let mut names = Vec::new();
    for (name, _) in agent::SUPPORTED_AGENTS {
        let _lock = LoginLock::acquire(name)?;
        let dir = login_dir(name)?;
        match fs::symlink_metadata(&dir) {
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("cannot inspect saved login {name}: {e}")),
            Ok(_) => {}
        }
        let root = open_private_root(&dir)?;
        if read_login_snapshot(name, &root, login_paths_for_agent(name))?
            .iter()
            .any(Option::is_some)
        {
            names.push(*name);
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
    let mut lock = LoginLock::acquire(agent_name)?;
    let data = open_private_root(&agentcage_data_dir()?)?;
    let target = directory_for_removal(&data, &format!("logins/{agent_name}"))?;
    // Record revocation outside the credential tree before deleting it.
    lock.revoke()?;
    let Some((_parent, anchored_path)) = target else {
        println!("no saved login for {agent_name}; older sessions cannot restore it");
        return Ok(());
    };
    match fs::remove_dir_all(&anchored_path) {
        Ok(()) => {
            println!("removed saved login: {agent_name}");
            Ok(())
        }
        Err(e) => Err(format!("cannot remove saved login '{}': {e}", agent_name)),
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
                agent, host_path, ..
            },
        ) => {
            format!("saved {agent} auth only ({})", host_path.display())
        }
        _ => "disposable".to_string(),
    };
    let host_home_line = match login_mount {
        LoginMount::Persistent { .. } => "not separately mounted; auth-only temp bind is mounted",
        LoginMount::None | LoginMount::Disabled => "not separately mounted",
    };
    eprintln!("AgentCage");
    eprintln!("  Workspace:  {} -> {}", cwd.display(), CONTAINER_PROJECT);
    eprintln!("  Home:       {home_line}");
    eprintln!("  Login:      {login_line}");
    eprintln!("  Host home:  {host_home_line}");
    eprintln!("  Network:    {network}");
    eprintln!("  Container:  auto-delete on exit");
    eprintln!();
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
    Ok(cage_dir)
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

    let lock = LoginLock::acquire(agent_name)?;
    let host_path = ensure_logins_dir()?.join(agent_name);
    create_private_dir_all(&host_path)?;
    harden_login_store(agent_name, &host_path, paths)?;
    let host_dir = open_private_root(&host_path)?;
    let baseline = read_login_snapshot(agent_name, &host_dir, paths)?;

    let staging_path = create_private_temp_dir(&format!("{LOGIN_STAGING_PREFIX}{agent_name}-"))?;
    let prepared = (|| {
        write_login_staging_owner(&staging_path)?;
        let staging_dir = open_private_root(&staging_path)?;
        for (rel, bytes) in paths.iter().zip(&baseline) {
            if let Some(bytes) = bytes {
                write_credential(&staging_dir, rel, bytes)?;
            }
        }
        Ok(staging_dir)
    })();
    let staging_dir = match prepared {
        Ok(dir) => dir,
        Err(e) => {
            let _ = fs::remove_dir_all(&staging_path);
            return Err(e);
        }
    };

    Ok(LoginMount::Persistent {
        agent: agent_name.to_string(),
        host_path,
        staging_path,
        staging_dir,
        paths,
        baseline,
        revocation: lock.revocation,
        retain_on_save_failure: Cell::new(false),
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

struct LoginLock {
    file: fs::File,
    revocation: u64,
}

impl LoginLock {
    fn acquire(agent: &str) -> Result<Self, String> {
        validate_login_agent(agent)?;
        let dir = agentcage_data_dir()?.join("login-locks");
        create_private_dir_all(&dir)?;
        let file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(dir.join(format!("{agent}.lock")))
            .map_err(|e| format!("cannot open login lock for {agent}: {e}"))?;
        let meta = file
            .metadata()
            .map_err(|e| format!("cannot inspect login lock: {e}"))?;
        if !meta.is_file() || meta.nlink() != 1 {
            return Err("refusing non-regular or hard-linked login lock".into());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("cannot secure login lock: {e}"))?;
        loop {
            if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) } == 0 {
                break;
            }
            let e = io::Error::last_os_error();
            if e.kind() != ErrorKind::Interrupted {
                return Err(format!("cannot lock saved login for {agent}: {e}"));
            }
        }
        let mut counter = String::new();
        (&file)
            .take(32)
            .read_to_string(&mut counter)
            .map_err(|e| format!("cannot read login revocation: {e}"))?;
        let revocation = if counter.is_empty() {
            0
        } else {
            counter
                .trim()
                .parse()
                .map_err(|_| "invalid login revocation counter".to_string())?
        };
        Ok(Self { file, revocation })
    }

    fn revoke(&mut self) -> Result<(), String> {
        let next = self
            .revocation
            .checked_add(1)
            .ok_or("login revocation counter exhausted")?;
        self.file
            .seek(SeekFrom::Start(0))
            .and_then(|_| self.file.write_all(format!("{next}\n").as_bytes()))
            .and_then(|_| self.file.set_len(format!("{next}\n").len() as u64))
            .and_then(|_| self.file.sync_all())
            .map_err(|e| format!("cannot revoke older login sessions: {e}"))?;
        self.revocation = next;
        Ok(())
    }
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
    open_directory_path(path, true)?
        .set_permissions(fs::Permissions::from_mode(0o700))
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

fn open_private_root(path: &Path) -> Result<fs::File, String> {
    open_directory_path(path, false)
}

fn open_directory_path(path: &Path, create: bool) -> Result<fs::File, String> {
    let mut dir = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(if path.is_absolute() {
            Path::new("/")
        } else {
            Path::new(".")
        })
        .map_err(|e| format!("cannot open private directory base: {e}"))?;
    for component in path.components() {
        let part = match component {
            Component::RootDir | Component::CurDir => continue,
            Component::Normal(part) => CString::new(part.as_bytes())
                .map_err(|_| "invalid private directory path".to_string())?,
            _ => {
                return Err(format!(
                    "invalid private directory path: {}",
                    path.display()
                ))
            }
        };
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
        if fd < 0 && create && io::Error::last_os_error().kind() == ErrorKind::NotFound {
            if unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) } < 0
                && io::Error::last_os_error().kind() != ErrorKind::AlreadyExists
            {
                return Err(format!(
                    "cannot create private directory {}: {}",
                    path.display(),
                    io::Error::last_os_error()
                ));
            }
            fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
        }
        if fd < 0 {
            return Err(format!(
                "cannot open private directory {} without following symlinks: {}",
                path.display(),
                io::Error::last_os_error()
            ));
        }
        dir = unsafe { fs::File::from_raw_fd(fd) };
    }
    Ok(dir)
}

fn directory_for_removal(
    root: &fs::File,
    rel: &str,
) -> Result<Option<(fs::File, PathBuf)>, String> {
    let Some((parent, leaf)) = relative_parent(root, rel, false)? else {
        return Ok(None);
    };
    let fd = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let error = io::Error::last_os_error();
        if error.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!("refusing unsafe or inaccessible state directory {rel} (symlinks are forbidden): {error}"));
    }
    let _selected = unsafe { fs::File::from_raw_fd(fd) };
    // Keep the inspected parent open. Rust's Linux remove_dir_all performs its
    // own descriptor-relative traversal and never follows nested symlinks.
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
        .join(std::ffi::OsStr::from_bytes(leaf.to_bytes()));
    Ok(Some((parent, anchored)))
}

fn relative_parent(
    root: &fs::File,
    rel: &str,
    create: bool,
) -> Result<Option<(fs::File, CString)>, String> {
    let path = Path::new(rel);
    let mut components: Vec<_> = path.components().collect();
    let Some(Component::Normal(leaf)) = components.pop() else {
        return Err(format!("invalid credential path: {rel}"));
    };
    let leaf = CString::new(leaf.as_bytes()).map_err(|_| "invalid credential path".to_string())?;
    let mut dir = root
        .try_clone()
        .map_err(|e| format!("cannot clone private directory: {e}"))?;
    for component in components {
        let Component::Normal(part) = component else {
            return Err(format!("invalid credential path: {rel}"));
        };
        let part =
            CString::new(part.as_bytes()).map_err(|_| "invalid credential path".to_string())?;
        let flags = libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC;
        let mut fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
        if fd < 0 && io::Error::last_os_error().kind() == ErrorKind::NotFound {
            if !create {
                return Ok(None);
            }
            let rc = unsafe { libc::mkdirat(dir.as_raw_fd(), part.as_ptr(), 0o700) };
            if rc < 0 && io::Error::last_os_error().kind() != ErrorKind::AlreadyExists {
                return Err(format!(
                    "cannot create credential directory for {rel}: {}",
                    io::Error::last_os_error()
                ));
            }
            fd = unsafe { libc::openat(dir.as_raw_fd(), part.as_ptr(), flags) };
        }
        if fd < 0 {
            return Err(format!("refusing unsafe or inaccessible credential parent for {rel} (symlinks are forbidden): {}", io::Error::last_os_error()));
        }
        dir = unsafe { fs::File::from_raw_fd(fd) };
        if create {
            dir.set_permissions(fs::Permissions::from_mode(0o700))
                .map_err(|e| format!("cannot secure credential directory for {rel}: {e}"))?;
        }
    }
    Ok(Some((dir, leaf)))
}

fn open_credential(root: &fs::File, rel: &str) -> Result<Option<fs::File>, String> {
    let Some((dir, leaf)) = relative_parent(root, rel, false)? else {
        return Ok(None);
    };
    let fd = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            leaf.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        let e = io::Error::last_os_error();
        if e.kind() == ErrorKind::NotFound {
            return Ok(None);
        }
        return Err(format!(
            "refusing unsafe or inaccessible login credential {rel}: {e}"
        ));
    }
    let file = unsafe { fs::File::from_raw_fd(fd) };
    let meta = file
        .metadata()
        .map_err(|e| format!("cannot inspect login credential {rel}: {e}"))?;
    if !meta.is_file() || meta.nlink() != 1 {
        return Err(format!(
            "refusing non-regular or hard-linked login credential {rel}"
        ));
    }
    if meta.len() > MAX_LOGIN_FILE_BYTES {
        return Err(format!(
            "refusing oversized login credential {rel} (max {MAX_LOGIN_FILE_BYTES} bytes)"
        ));
    }
    Ok(Some(file))
}

fn read_credential(root: &fs::File, rel: &str) -> Result<Option<Vec<u8>>, String> {
    let Some(file) = open_credential(root, rel)? else {
        return Ok(None);
    };
    read_open_credential(file, rel).map(Some)
}

fn read_open_credential(file: fs::File, rel: &str) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    file.take(MAX_LOGIN_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("cannot read login credential {rel}: {e}"))?;
    if bytes.len() as u64 > MAX_LOGIN_FILE_BYTES {
        return Err(format!("refusing oversized login credential {rel}"));
    }
    Ok(bytes)
}

fn write_credential(root: &fs::File, rel: &str, bytes: &[u8]) -> Result<(), String> {
    let (dir, leaf) = relative_parent(root, rel, true)?.ok_or("missing credential parent")?;
    for _ in 0..100 {
        let temp =
            CString::new(format!(".agentcage-tmp-{}-{}", process::id(), nanoid(12))).unwrap();
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                temp.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd < 0 {
            let e = io::Error::last_os_error();
            if e.kind() == ErrorKind::AlreadyExists {
                continue;
            }
            return Err(format!(
                "cannot create credential temporary file for {rel}: {e}"
            ));
        }
        let mut file = unsafe { fs::File::from_raw_fd(fd) };
        let result = file
            .write_all(bytes)
            .and_then(|_| file.sync_all())
            .and_then(|_| {
                if unsafe {
                    libc::renameat(
                        dir.as_raw_fd(),
                        temp.as_ptr(),
                        dir.as_raw_fd(),
                        leaf.as_ptr(),
                    )
                } == 0
                {
                    Ok(())
                } else {
                    Err(io::Error::last_os_error())
                }
            })
            .and_then(|_| dir.sync_all());
        if result.is_err() {
            unsafe {
                libc::unlinkat(dir.as_raw_fd(), temp.as_ptr(), 0);
            }
        }
        return result.map_err(|e| format!("cannot save login credential {rel}: {e}"));
    }
    Err(format!(
        "cannot create unique credential temporary file for {rel}"
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
    let dir = open_private_root(root)?;
    for rel in paths {
        if let Some(file) = open_credential(&dir, rel)? {
            // fchmod changes the inspected inode, never a reopened pathname.
            relative_parent(&dir, rel, true)?;
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("cannot secure saved {agent_name} login {rel}: {e}"))?;
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
    let root = open_private_root(home_path)?;
    root.set_permissions(fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("cannot secure persistent home: {e}"))?;
    for (agent_name, _) in agent::SUPPORTED_AGENTS {
        for rel in login_paths_for_agent(agent_name) {
            if let Some(file) = open_credential(&root, rel)? {
                relative_parent(&root, rel, true)?;
                file.set_permissions(fs::Permissions::from_mode(0o600))
                    .map_err(|e| format!("cannot secure {agent_name} login {rel}: {e}"))?;
            }
        }
    }
    Ok(())
}

fn read_login_snapshot(
    agent: &str,
    root: &fs::File,
    paths: &[&str],
) -> Result<Vec<Option<Vec<u8>>>, String> {
    paths
        .iter()
        .map(|rel| {
            let Some(bytes) = read_credential(root, rel)? else {
                return Ok(None);
            };
            if should_persist_login_bytes(agent, rel, &bytes)? {
                Ok(Some(bytes))
            } else {
                Ok(None)
            }
        })
        .collect()
}

fn should_persist_login_bytes(agent: &str, rel: &str, bytes: &[u8]) -> Result<bool, String> {
    if agent == "antigravity" {
        return Ok(!bytes.is_empty());
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| format!("invalid UTF-8 in login credential {rel}"))?;
    let value =
        JsonParser::parse(text).map_err(|_| format!("invalid JSON in login credential {rel}"))?;
    let invalid_shape =
        || format!("invalid credential structure in {rel}; saved login was not changed");
    let JsonValue::Object(fields) = value else {
        return Err(invalid_shape());
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    match (agent, rel) {
        ("codex", "auth.json" | ".codex/auth.json") => {
            codex_auth_material(&fields).ok_or_else(invalid_shape)
        }
        ("claude", ".claude.json") => match field("primaryApiKey") {
            None => Ok(false),
            Some(JsonValue::String(key)) => Ok(!key.is_empty()),
            _ => Err(invalid_shape()),
        },
        ("claude", ".claude/.credentials.json") => match field("claudeAiOauth") {
            None => Ok(false),
            Some(JsonValue::Object(tokens)) => {
                claude_auth_material(tokens).ok_or_else(invalid_shape)
            }
            _ => Err(invalid_shape()),
        },
        ("opencode", ".local/share/opencode/account.json") => {
            opencode_account_material(&fields).ok_or_else(invalid_shape)
        }
        ("opencode", ".local/share/opencode/auth.json") => {
            for (_, auth) in &fields {
                let JsonValue::Object(tokens) = auth else {
                    return Err(invalid_shape());
                };
                if !valid_opencode_provider(tokens) {
                    return Err(invalid_shape());
                }
            }
            Ok(!fields.is_empty())
        }
        _ => Err(invalid_shape()),
    }
}

#[derive(Debug)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(String),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

fn json_field<'a>(fields: &'a [(String, JsonValue)], name: &str) -> Option<&'a JsonValue> {
    fields
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

fn json_string<'a>(fields: &'a [(String, JsonValue)], name: &str) -> Option<&'a str> {
    match json_field(fields, name) {
        Some(JsonValue::String(value)) => Some(value),
        _ => None,
    }
}

fn optional_json_string(fields: &[(String, JsonValue)], name: &str) -> bool {
    matches!(
        json_field(fields, name),
        None | Some(JsonValue::Null | JsonValue::String(_))
    )
}

fn nonnegative_json_integer(value: &JsonValue) -> bool {
    matches!(value, JsonValue::Number(text) if text.parse::<f64>().is_ok_and(|number| {
        number.is_finite() && number >= 0.0 && number.fract() == 0.0
    }))
}

// Pinned Claude 2.1.289 writes these OAuth fields, and clears access/refresh
// tokens on invalid_grant. An auth-free configuration remains a valid logout.
fn claude_auth_material(fields: &[(String, JsonValue)]) -> Option<bool> {
    let access = json_string(fields, "accessToken")?;
    for key in [
        "refreshToken",
        "clientId",
        "subscriptionType",
        "rateLimitTier",
    ] {
        if !optional_json_string(fields, key) {
            return None;
        }
    }
    for key in ["expiresAt", "refreshTokenExpiresAt"] {
        if let Some(value) = json_field(fields, key) {
            if !matches!(value, JsonValue::Null) && !nonnegative_json_integer(value) {
                return None;
            }
        }
    }
    match json_field(fields, "scopes") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Array(values))
            if values.iter().all(|v| matches!(v, JsonValue::String(_))) => {}
        _ => return None,
    }
    Some(!access.is_empty() || json_string(fields, "refreshToken").is_some_and(|v| !v.is_empty()))
}

// AuthDotJson and TokenData from pinned Codex rust-v0.160.0, login/src/auth/
// storage.rs and login/src/token_data.rs. This checks the stored format, never
// the token's signature, expiry against the clock, or provider acceptance.
fn codex_auth_material(fields: &[(String, JsonValue)]) -> Option<bool> {
    match json_field(fields, "auth_mode") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::String(mode))
            if matches!(
                mode.as_str(),
                "apikey"
                    | "chatgpt"
                    | "chatgptAuthTokens"
                    | "headers"
                    | "agentIdentity"
                    | "personalAccessToken"
                    | "bedrockApiKey"
                    | "bedrockAccessKeys"
            ) => {}
        _ => return None,
    }
    let mut material = false;
    for key in ["OPENAI_API_KEY", "personal_access_token"] {
        if !optional_json_string(fields, key) {
            return None;
        }
        material |= json_string(fields, key).is_some_and(|value| !value.is_empty());
    }
    match json_field(fields, "last_refresh") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::String(value)) if valid_auth_timestamp(value) => {}
        _ => return None,
    }
    match json_field(fields, "tokens") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::Object(tokens)) => {
            let id = json_string(tokens, "id_token")?;
            let access = json_string(tokens, "access_token")?;
            let refresh = json_string(tokens, "refresh_token")?;
            if !valid_codex_id_token(id) || !optional_json_string(tokens, "account_id") {
                return None;
            }
            material |= !access.is_empty() || !refresh.is_empty();
        }
        _ => return None,
    }
    match json_field(fields, "agent_identity") {
        None | Some(JsonValue::Null) => {}
        Some(JsonValue::String(token)) => material |= !token.trim().is_empty(),
        Some(JsonValue::Object(identity)) => {
            for key in [
                "agent_runtime_id",
                "agent_private_key",
                "account_id",
                "chatgpt_user_id",
                "plan_type",
            ] {
                json_string(identity, key)?;
            }
            if !optional_json_string(identity, "email")
                || !optional_json_string(identity, "task_id")
                || !matches!(
                    json_field(identity, "chatgpt_account_is_fedramp"),
                    Some(JsonValue::Bool(_))
                )
            {
                return None;
            }
            material |= !json_string(identity, "agent_runtime_id")?.is_empty()
                && !json_string(identity, "agent_private_key")?.is_empty();
        }
        _ => return None,
    }
    for (key, required) in [
        ("bedrock_api_key", &["api_key", "region"][..]),
        (
            "bedrock_access_keys",
            &["access_key_id", "secret_access_key"][..],
        ),
    ] {
        match json_field(fields, key) {
            None | Some(JsonValue::Null) => {}
            Some(JsonValue::Object(auth)) => {
                for key in required {
                    json_string(auth, key)?;
                }
                if key == "bedrock_access_keys" && !optional_json_string(auth, "session_token") {
                    return None;
                }
                material |= required
                    .iter()
                    .all(|key| json_string(auth, key).is_some_and(|v| !v.is_empty()));
            }
            _ => return None,
        }
    }
    Some(material)
}

fn valid_codex_id_token(token: &str) -> bool {
    let mut parts = token.split('.');
    if !parts.next().is_some_and(|p| !p.is_empty()) {
        return false;
    }
    let Some(payload) = parts.next().filter(|p| !p.is_empty()) else {
        return false;
    };
    if !parts.next().is_some_and(|p| !p.is_empty()) || payload.len() % 4 == 1 {
        return false;
    }
    // URL_SAFE_NO_PAD, including its zero trailing-bit requirement.
    let mut decoded = Vec::with_capacity(payload.len() * 3 / 4);
    let (mut bits, mut count) = (0u32, 0);
    for byte in payload.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return false,
        };
        bits = (bits << 6) | u32::from(value);
        count += 6;
        if count >= 8 {
            count -= 8;
            decoded.push((bits >> count) as u8);
            bits &= (1 << count) - 1;
        }
    }
    if bits != 0 {
        return false;
    }
    let Ok(text) = std::str::from_utf8(&decoded) else {
        return false;
    };
    let Ok(JsonValue::Object(claims)) = JsonParser::parse(text) else {
        return false;
    };
    if !optional_json_string(&claims, "email") {
        return false;
    }
    for (key, names) in [
        ("https://api.openai.com/profile", &["email"][..]),
        (
            "https://api.openai.com/auth",
            &[
                "chatgpt_plan_type",
                "chatgpt_user_id",
                "user_id",
                "chatgpt_account_id",
            ][..],
        ),
    ] {
        match json_field(&claims, key) {
            None | Some(JsonValue::Null) => {}
            Some(JsonValue::Object(values)) => {
                if !names.iter().all(|name| optional_json_string(values, name)) {
                    return false;
                }
                if key.ends_with("/auth")
                    && !matches!(
                        json_field(values, "chatgpt_account_is_fedramp"),
                        None | Some(JsonValue::Bool(_))
                    )
                {
                    return false;
                }
            }
            _ => return false,
        }
    }
    true
}

fn valid_auth_timestamp(value: &str) -> bool {
    let Some(separator) = value.find(['T', 't', ' ']) else {
        return false;
    };
    let (date, time) = value.split_at(separator);
    let Some((year_month, day)) = date.rsplit_once('-') else {
        return false;
    };
    let Some((year, month)) = year_month.rsplit_once('-') else {
        return false;
    };
    let (Ok(year), Ok(month), Ok(day)) = (
        year.parse::<i32>(),
        month.parse::<u32>(),
        day.parse::<u32>(),
    ) else {
        return false;
    };
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => return false,
    };
    let bytes = time[1..].as_bytes();
    let pair = |position| {
        bytes.get(position..position + 2).and_then(|pair: &[u8]| {
            pair.iter()
                .all(u8::is_ascii_digit)
                .then(|| u32::from(pair[0] - b'0') * 10 + u32::from(pair[1] - b'0'))
        })
    };
    if day == 0
        || day > days
        || bytes.get(2) != Some(&b':')
        || bytes.get(5) != Some(&b':')
        || !pair(0).is_some_and(|v| v < 24)
        || !pair(3).is_some_and(|v| v < 60)
        || !pair(6).is_some_and(|v| v <= 60)
    {
        return false;
    }
    let mut position = 8;
    if bytes.get(position) == Some(&b'.') {
        position += 1;
        let start = position;
        while bytes.get(position).is_some_and(u8::is_ascii_digit) {
            position += 1;
        }
        if start == position {
            return false;
        }
    }
    match bytes.get(position..) {
        Some(b"Z" | b"z") => true,
        Some(zone) if zone.len() == 6 && matches!(zone[0], b'+' | b'-') && zone[3] == b':' => {
            pair(position + 1).is_some_and(|v| v < 24) && pair(position + 4).is_some_and(|v| v < 60)
        }
        _ => false,
    }
}

// Match the exact auth-only document accepted by the embedded SQLite bridge.
fn opencode_account_material(fields: &[(String, JsonValue)]) -> Option<bool> {
    if fields.len() != 3 {
        return None;
    }
    let JsonValue::Array(accounts) = json_field(fields, "accounts")? else {
        return None;
    };
    if accounts.len() > 10_000 {
        return None;
    }
    let mut identifiers = std::collections::HashSet::with_capacity(accounts.len());
    for account in accounts {
        let JsonValue::Object(row) = account else {
            return None;
        };
        if row.len() != 6
            || ["id", "email", "url", "access_token", "refresh_token"]
                .iter()
                .any(|key| json_string(row, key).is_none_or(str::is_empty))
        {
            return None;
        }
        match json_field(row, "token_expiry")? {
            JsonValue::Null => {}
            // Python json.loads distinguishes integers from floats. The
            // bridge accepts only nullable, nonnegative signed-64-bit ints.
            JsonValue::Number(number) if number.parse::<i64>().is_ok_and(|n| n >= 0) => {}
            _ => return None,
        }
        if !identifiers.insert(json_string(row, "id")?) {
            return None;
        }
    }
    let active = match json_field(fields, "active_account_id")? {
        JsonValue::Null => None,
        JsonValue::String(id) if identifiers.contains(id.as_str()) => Some(id),
        _ => return None,
    };
    match json_field(fields, "active_org_id")? {
        JsonValue::Null => {}
        JsonValue::String(org) if !org.is_empty() && active.is_some() => {}
        _ => return None,
    }
    Some(!accounts.is_empty())
}

// Provider credential variants from the pinned OpenCode v1.18.34
// packages/opencode/src/auth/index.ts. Logout removes the provider entry.
fn valid_opencode_provider(fields: &[(String, JsonValue)]) -> bool {
    let string = |key| matches!(json_field(fields, key), Some(JsonValue::String(_)));
    let optional_string = |key| json_field(fields, key).is_none() || string(key);
    match json_field(fields, "type") {
        Some(JsonValue::String(kind)) if kind == "api" => {
            string("key")
                && match json_field(fields, "metadata") {
                    None => true,
                    Some(JsonValue::Object(values)) => values
                        .iter()
                        .all(|(_, value)| matches!(value, JsonValue::String(_))),
                    _ => false,
                }
        }
        Some(JsonValue::String(kind)) if kind == "oauth" => {
            string("access")
                && string("refresh")
                && optional_string("accountId")
                && optional_string("enterpriseUrl")
                && match json_field(fields, "expires") {
                    Some(JsonValue::Number(value)) => value.parse::<f64>().is_ok_and(|number| {
                        number.is_finite() && number >= 0.0 && number.fract() == 0.0
                    }),
                    _ => false,
                }
        }
        Some(JsonValue::String(kind)) if kind == "wellknown" => string("key") && string("token"),
        _ => false,
    }
}

// Limited credential and Podman-info JSON: bounded nesting and item count,
// complete syntax validation, no policy/configuration language.
struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
    items: usize,
}

impl<'a> JsonParser<'a> {
    fn parse(text: &'a str) -> Result<JsonValue, ()> {
        let mut parser = Self {
            bytes: text.as_bytes(),
            pos: 0,
            items: 0,
        };
        let value = parser.value(0)?;
        parser.space();
        if parser.pos != parser.bytes.len() {
            return Err(());
        }
        Ok(value)
    }

    fn space(&mut self) {
        while matches!(self.bytes.get(self.pos), Some(b' ' | b'\n' | b'\r' | b'\t')) {
            self.pos += 1;
        }
    }

    fn take(&mut self, byte: u8) -> bool {
        self.space();
        if self.bytes.get(self.pos) == Some(&byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn literal(&mut self, expected: &[u8], value: JsonValue) -> Result<JsonValue, ()> {
        if self.bytes.get(self.pos..self.pos + expected.len()) != Some(expected) {
            return Err(());
        }
        self.pos += expected.len();
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<JsonValue, ()> {
        self.items += 1;
        if depth > 64 || self.items > 100_000 {
            return Err(());
        }
        self.space();
        match self.bytes.get(self.pos) {
            Some(b'n') => self.literal(b"null", JsonValue::Null),
            Some(b't') => self.literal(b"true", JsonValue::Bool(true)),
            Some(b'f') => self.literal(b"false", JsonValue::Bool(false)),
            Some(b'"') => self.string().map(JsonValue::String),
            Some(b'[') => {
                self.pos += 1;
                let mut items = Vec::new();
                if self.take(b']') {
                    return Ok(JsonValue::Array(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    if self.take(b']') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(());
                    }
                }
                Ok(JsonValue::Array(items))
            }
            Some(b'{') => {
                self.pos += 1;
                let mut fields = Vec::new();
                let mut keys = std::collections::HashSet::new();
                if self.take(b'}') {
                    return Ok(JsonValue::Object(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    if !keys.insert(key.clone()) {
                        return Err(());
                    }
                    if !self.take(b':') {
                        return Err(());
                    }
                    fields.push((key, self.value(depth + 1)?));
                    if self.take(b'}') {
                        break;
                    }
                    if !self.take(b',') {
                        return Err(());
                    }
                }
                Ok(JsonValue::Object(fields))
            }
            Some(b'-' | b'0'..=b'9') => {
                let number_start = self.pos;
                if self.bytes.get(self.pos) == Some(&b'-') {
                    self.pos += 1;
                }
                match self.bytes.get(self.pos) {
                    Some(b'0') => self.pos += 1,
                    Some(b'1'..=b'9') => {
                        while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                            self.pos += 1;
                        }
                    }
                    _ => return Err(()),
                }
                if self.bytes.get(self.pos) == Some(&b'.') {
                    self.pos += 1;
                    let start = self.pos;
                    while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                        self.pos += 1;
                    }
                    if self.pos == start {
                        return Err(());
                    }
                }
                if matches!(self.bytes.get(self.pos), Some(b'e' | b'E')) {
                    self.pos += 1;
                    if matches!(self.bytes.get(self.pos), Some(b'+' | b'-')) {
                        self.pos += 1;
                    }
                    let start = self.pos;
                    while matches!(self.bytes.get(self.pos), Some(b'0'..=b'9')) {
                        self.pos += 1;
                    }
                    if self.pos == start {
                        return Err(());
                    }
                }
                Ok(JsonValue::Number(
                    std::str::from_utf8(&self.bytes[number_start..self.pos])
                        .map_err(|_| ())?
                        .to_string(),
                ))
            }
            _ => Err(()),
        }
    }

    fn hex4(&mut self) -> Result<u32, ()> {
        let digits = self.bytes.get(self.pos..self.pos + 4).ok_or(())?;
        if !digits.iter().all(u8::is_ascii_hexdigit) {
            return Err(());
        }
        let text = std::str::from_utf8(digits).map_err(|_| ())?;
        let value = u32::from_str_radix(text, 16).map_err(|_| ())?;
        self.pos += 4;
        Ok(value)
    }

    fn string(&mut self) -> Result<String, ()> {
        if self.bytes.get(self.pos) != Some(&b'"') {
            return Err(());
        }
        self.pos += 1;
        let mut result = String::new();
        loop {
            match self.bytes.get(self.pos).copied().ok_or(())? {
                b'"' => {
                    self.pos += 1;
                    return Ok(result);
                }
                b'\\' => {
                    self.pos += 1;
                    let escaped = self.bytes.get(self.pos).copied().ok_or(())?;
                    self.pos += 1;
                    result.push(match escaped {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\x08',
                        b'f' => '\x0c',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xd800..=0xdbff).contains(&code) {
                                if self.bytes.get(self.pos..self.pos + 2) != Some(b"\\u") {
                                    return Err(());
                                }
                                self.pos += 2;
                                let low = self.hex4()?;
                                if !(0xdc00..=0xdfff).contains(&low) {
                                    return Err(());
                                }
                                code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                            }
                            char::from_u32(code).ok_or(())?
                        }
                        _ => return Err(()),
                    });
                }
                0..=31 => return Err(()),
                _ => {
                    let start = self.pos;
                    while self
                        .bytes
                        .get(self.pos)
                        .map(|b| *b >= 32 && *b != b'"' && *b != b'\\')
                        .unwrap_or(false)
                    {
                        self.pos += 1;
                    }
                    result.push_str(
                        std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| ())?,
                    );
                }
            }
        }
    }
}

fn check_login_tree(root: &fs::File, prefix: &str, paths: &[&str]) -> Result<(), String> {
    let entries = fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd()))
        .map_err(|e| format!("cannot inspect saved login directory: {e}"))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot inspect saved login entry: {e}"))?;
        let name = entry.file_name();
        let name = name
            .to_str()
            .ok_or("unrecognized saved login filename; store was kept")?;
        let rel = format!("{prefix}{name}");
        if paths.contains(&rel.as_str()) {
            // Keep file/path validation on the same descriptor-relative path.
            open_credential(root, name)?.ok_or("saved login changed during inspection")?;
        } else if paths
            .iter()
            .any(|path| path.starts_with(&format!("{rel}/")))
        {
            let (child, _) = relative_parent(root, &format!("{name}/placeholder"), false)?
                .ok_or("saved login directory changed during inspection")?;
            check_login_tree(&child, &format!("{rel}/"), paths)?;
        } else {
            return Err(format!(
                "unrecognized saved login entry {rel}; store was kept"
            ));
        }
    }
    Ok(())
}

fn publish_login_snapshot(
    host_path: &Path,
    host: &fs::File,
    paths: &[&str],
    updated: &[Option<Vec<u8>>],
) -> Result<(), String> {
    // Build and fsync a complete sibling first. One atomic directory exchange
    // then publishes all credentials/deletions together, including on ENOSPC.
    // Refuse unexpected store contents rather than delete unmanaged files.
    check_login_tree(host, "", paths)?;
    let parent = open_private_root(host_path.parent().ok_or("missing login store parent")?)?;
    let leaf = CString::new(
        host_path
            .file_name()
            .ok_or("missing login store name")?
            .as_bytes(),
    )
    .map_err(|_| "invalid login store name")?;
    let temp = CString::new(format!(".agentcage-save-{}-{}", process::id(), nanoid(16))).unwrap();
    if unsafe { libc::mkdirat(parent.as_raw_fd(), temp.as_ptr(), 0o700) } != 0 {
        return Err(format!(
            "cannot prepare login snapshot: {}",
            io::Error::last_os_error()
        ));
    }
    let anchored = PathBuf::from(format!("/proc/self/fd/{}", parent.as_raw_fd()))
        .join(std::ffi::OsStr::from_bytes(temp.to_bytes()));
    let mut keep_backup = false;
    let result = (|| {
        let (prepared, _) = relative_parent(
            &parent,
            &format!("{}/placeholder", temp.to_str().unwrap()),
            false,
        )?
        .ok_or("prepared login snapshot disappeared")?;
        for (rel, bytes) in paths.iter().zip(updated) {
            if let Some(bytes) = bytes {
                write_credential(&prepared, rel, bytes)?;
            }
        }
        prepared
            .sync_all()
            .map_err(|e| format!("cannot sync prepared login snapshot: {e}"))?;
        let mut selected: libc::stat = unsafe { std::mem::zeroed() };
        let original = host
            .metadata()
            .map_err(|e| format!("cannot inspect saved login: {e}"))?;
        if unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                leaf.as_ptr(),
                &mut selected,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
            || selected.st_dev != original.dev()
            || selected.st_ino != original.ino()
        {
            return Err("saved login directory changed before commit; store was kept".into());
        }
        let exchange = || unsafe {
            libc::renameat2(
                parent.as_raw_fd(),
                temp.as_ptr(),
                parent.as_raw_fd(),
                leaf.as_ptr(),
                libc::RENAME_EXCHANGE,
            )
        };
        if exchange() != 0 {
            return Err(format!(
                "cannot commit login snapshot: {}",
                io::Error::last_os_error()
            ));
        }
        if let Err(error) = parent.sync_all() {
            if exchange() != 0 {
                keep_backup = true;
                return Err(format!("login commit sync failed: {error}; rollback failed: {}; previous snapshot retained at {}",
                    io::Error::last_os_error(), host_path.parent().unwrap().join(std::ffi::OsStr::from_bytes(temp.to_bytes())).display()));
            }
            let _ = parent.sync_all();
            return Err(format!(
                "login commit sync failed; previous snapshot restored: {error}"
            ));
        }
        Ok(())
    })();
    if !keep_backup {
        if let Err(error) = fs::remove_dir_all(&anchored) {
            return Err(match result {
                Ok(()) => {
                    format!("login snapshot saved, but previous snapshot cleanup failed: {error}")
                }
                Err(original) => format!("{original}; temporary snapshot cleanup failed: {error}"),
            });
        }
    }
    result
}

fn finalize_login_mount(login_mount: &LoginMount) -> Result<(), String> {
    let LoginMount::Persistent {
        agent,
        host_path,
        staging_dir,
        paths,
        baseline,
        revocation,
        staging_path,
        retain_on_save_failure,
    } = login_mount
    else {
        return Ok(());
    };
    let mut validated_change = false;
    let result = (|| {
        // Podman/agent exit status cannot distinguish a failed copy-out from
        // a successful export after a nonzero agent exit. Accept a snapshot
        // only after the container helper completed every write and deletion.
        let receipt = open_credential(staging_dir, LOGIN_STAGING_COMPLETE_FILE)?
            .ok_or("credential export did not complete; the saved login store was kept")?;
        let mut completion = Vec::new();
        receipt
            .take(9)
            .read_to_end(&mut completion)
            .map_err(|e| format!("cannot read credential export completion: {e}"))?;
        if completion != b"complete\n" {
            return Err(
                "invalid credential export completion; the saved login store was kept".into(),
            );
        }
        let lock = LoginLock::acquire(agent)?;
        if lock.revocation != *revocation {
            return Err(format!("login persistence skipped: saved {agent} login was removed while this session was running; start a new session to log in again"));
        }
        // Validate every source before touching the store. Invalid paths/content
        // must never be interpreted as a request to delete existing credentials.
        let updated = read_login_snapshot(agent, staging_dir, paths)?;
        if updated == *baseline {
            return Ok(());
        }
        validated_change = true;
        create_private_dir_all(host_path)?;
        let host = open_private_root(host_path)?;
        let current = read_login_snapshot(agent, &host, paths)?;
        if current == updated {
            return Ok(());
        }
        if current != *baseline {
            return Err(format!("login persistence conflict for {agent}: another session changed the saved login; the newer store was kept"));
        }
        publish_login_snapshot(host_path, &host, paths, &updated)
    })();
    let result = match result {
        Err(error) if validated_change => {
            retain_on_save_failure.set(true);
            // A non-PID owner marker conservatively excludes this private,
            // validated recovery copy from automatic stale staging cleanup.
            let marker = write_credential(staging_dir, LOGIN_STAGING_OWNER_FILE, b"save-failed");
            Err(format!("{error}; returned credentials retained for recovery at {}{}",
                staging_path.display(), marker.err().map(|error| format!("; cannot mark recovery copy: {error}; normal 24-hour cleanup still applies")).unwrap_or_default()))
        }
        result => result,
    };
    result.map_err(|error| format!("failed to save login credentials for {agent}: {error}"))
}
fn cleanup_login_staging(login_mount: &LoginMount) -> Result<(), String> {
    if let LoginMount::Persistent {
        staging_path,
        retain_on_save_failure,
        ..
    } = login_mount
    {
        if retain_on_save_failure.get() {
            return Ok(());
        }
        match fs::remove_dir_all(staging_path) {
            Ok(()) => {}
            Err(e) if e.kind() == ErrorKind::NotFound => {}
            Err(e) => {
                return Err(format!(
                    "cannot remove login staging {}: {e}",
                    staging_path.display()
                ))
            }
        }
    }
    Ok(())
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
    let root = open_private_root(staging_path)?;
    write_credential(
        &root,
        LOGIN_STAGING_OWNER_FILE,
        process::id().to_string().as_bytes(),
    )
}

fn login_staging_owner_alive(staging_path: &Path) -> bool {
    // Unsafe or unreadable owner metadata must not block on a FIFO or permit
    // deletion of a directory that may still belong to an active session.
    let Ok(root) = open_private_root(staging_path) else {
        return true;
    };
    let file = match open_credential(&root, LOGIN_STAGING_OWNER_FILE) {
        Ok(Some(file)) => file,
        Ok(None) => return false,
        Err(_) => return true,
    };
    let mut content = Vec::new();
    if file.take(33).read_to_end(&mut content).is_err() || content.len() > 32 {
        return true;
    }
    let Ok(content) = std::str::from_utf8(&content) else {
        return true;
    };
    let Ok(pid) = content.trim().parse::<libc::pid_t>() else {
        return true;
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

fn build_run_argv(
    cwd: &Path,
    command: &str,
    extra_args: &[OsString],
    home_mount: &HomeMount,
    login_mount: &LoginMount,
) -> Result<(Vec<OsString>, String), String> {
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

    argv.push("-i".to_string());
    if has_terminal() {
        argv.push("-t".to_string());
    }

    argv.extend([
        "--name".to_string(),
        container_name.clone(),
        "--label".to_string(),
        CONTAINER_OWNER_LABEL.to_string(),
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
            "type=tmpfs,destination={CONTAINER_CAGE_DIR},tmpfs-size=64M,tmpfs-mode=0700,U=true,notmpcopyup"
        ),
        "--workdir".to_string(),
        CONTAINER_PROJECT.to_string(),
    ]);

    match home_mount {
        HomeMount::Disposable => {
            argv.push("--mount".to_string());
            argv.push(format!(
                "type=tmpfs,destination={CONTAINER_HOME},tmpfs-size=512M,tmpfs-mode=0700,U=true,notmpcopyup"
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

    for (key, value) in RUNTIME_ENV_VARS {
        argv.push("-e".to_string());
        argv.push(format!("{key}={value}"));
    }

    argv.push(IMAGE.to_string());
    match login_mount {
        LoginMount::Persistent { agent, paths, .. } => {
            argv.push("bash".to_string());
            argv.push("-c".to_string());
            argv.push(LOGIN_SYNC_SCRIPT.to_string());
            argv.push("agentcage-login-sync".to_string());
            argv.push(CONTAINER_LOGIN_SYNC.to_string());
            argv.push(MAX_LOGIN_FILE_BYTES.to_string());
            argv.push(paths.len().to_string());
            argv.extend(paths.iter().map(|path| path.to_string()));
            argv.push(
                if agent == "opencode" {
                    OPENCODE_ACCOUNT_BRIDGE
                } else {
                    ""
                }
                .to_string(),
            );
            argv.push(LOGIN_FILE_SYNC.to_string());
        }
        LoginMount::None | LoginMount::Disabled => {}
    }

    let mut argv: Vec<OsString> = argv.into_iter().map(OsString::from).collect();
    if extra_args.iter().any(|arg| arg.to_str().is_none()) {
        argv.extend(["python3", "-c", OPAQUE_ARGV_EXEC, command].map(OsString::from));
        for arg in extra_args {
            let chunks = arg.as_bytes().chunks(32 * 1024);
            argv.push(chunks.len().to_string().into());
            for chunk in chunks {
                let encoded: String = chunk.iter().map(|byte| format!("{byte:02x}")).collect();
                argv.push(encoded.into());
            }
        }
    } else {
        argv.push(command.into());
        argv.extend_from_slice(extra_args);
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
    argv: &[OsString],
    container_name: &str,
    login_mount: &LoginMount,
) -> Result<i32, String> {
    let Some((program, args)) = argv.split_first() else {
        return Err("empty command".into());
    };
    let signals = SignalForwarder::new()?;

    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::inherit())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start podman: {e}"))?;
    signals.attach(child.id() as i32);

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    let stdout_thread = thread::spawn(move || forward_stream(stdout, io::stdout()));
    let stderr_thread = thread::spawn(move || forward_stream(stderr, io::stderr()));

    let status = match child.wait() {
        Ok(status) => status,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = stdout_thread.join();
            let _ = stderr_thread.join();
            return Err(format!("podman exited with error: {e}"));
        }
    };
    PODMAN_CHILD_PID.store(0, Ordering::SeqCst);

    let stdout_result = stdout_thread
        .join()
        .map_err(|_| "stdout forwarding thread failed".to_string())
        .and_then(|result| result.map_err(|e| format!("cannot forward container stdout: {e}")));
    let stderr_result = stderr_thread
        .join()
        .map_err(|_| "stderr forwarding thread failed".to_string())
        .and_then(|result| result.map_err(|e| format!("cannot forward container stderr: {e}")));

    let finalization = finalize_login_mount(login_mount);
    let mut code = status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(1));
    let interrupted = FORWARDED_SIGNAL.load(Ordering::SeqCst);
    if code == 0 && interrupted != 0 {
        code = 128 + interrupted;
    }
    // --rm owns normal teardown. Never force-remove a possibly running container.
    if code != 0 {
        eprintln!("container {container_name} exited with status {code}");
    }
    let result = combine_session_result(Ok(code), finalization);
    combine_session_result(result, stdout_result.and(stderr_result))
}

static PODMAN_CHILD_PID: AtomicI32 = AtomicI32::new(0);
static FORWARDED_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(signal: libc::c_int) {
    FORWARDED_SIGNAL.store(signal, Ordering::SeqCst);
    let pid = PODMAN_CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        unsafe {
            libc::kill(pid, signal);
        }
    }
}

struct SignalForwarder {
    previous: Vec<(i32, libc::sigaction)>,
}

impl SignalForwarder {
    fn new() -> Result<Self, String> {
        PODMAN_CHILD_PID.store(0, Ordering::SeqCst);
        FORWARDED_SIGNAL.store(0, Ordering::SeqCst);
        let mut guard = Self {
            previous: Vec::new(),
        };
        for signal in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP] {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = forward_signal as *const () as usize;
            unsafe {
                libc::sigemptyset(&mut action.sa_mask);
            }
            let mut previous = unsafe { std::mem::zeroed() };
            if unsafe { libc::sigaction(signal, &action, &mut previous) } != 0 {
                return Err(format!(
                    "cannot install session signal handler: {}",
                    io::Error::last_os_error()
                ));
            }
            guard.previous.push((signal, previous));
        }
        Ok(guard)
    }

    fn attach(&self, pid: i32) {
        PODMAN_CHILD_PID.store(pid, Ordering::SeqCst);
        let pending = FORWARDED_SIGNAL.load(Ordering::SeqCst);
        if pending != 0 {
            unsafe {
                libc::kill(pid, pending);
            }
        }
    }
}

impl Drop for SignalForwarder {
    fn drop(&mut self) {
        PODMAN_CHILD_PID.store(0, Ordering::SeqCst);
        for (signal, previous) in &self.previous {
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}

fn forward_stream<R: Read, W: io::Write>(mut reader: R, mut writer: W) -> io::Result<()> {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => return Ok(()),
            Ok(n) => {
                writer.write_all(&buf[..n])?;
                writer.flush()?;
            }
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
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
    eprintln!("AgentCage — checking Podman runtime and image...");
    let executable = find_podman()?;
    let version = podman_version(&executable)?;
    if !version_at_least(&version, [4, 9, 3]) {
        return Err(format!("Podman 4.9.3 or newer is required; found '{version}'. Upgrade Podman before running {CLI}."));
    }
    let PodmanHost {
        rootless,
        remote,
        runtime,
        runtime_version,
        cgroups,
        manager,
        controllers,
    } = podman_host(&executable)?;
    if !rootless {
        return Err("Rootless Podman is required. Run `podman info --format '{{.Host.Security.Rootless}}'` and configure Podman rootless mode.".into());
    }
    if remote {
        return Err("Local Podman is required: remote services cannot mount the caller's project and login staging paths. Use a local rootless Podman connection.".into());
    }
    if runtime != "crun" {
        return Err(format!("crun is required for --group-add=keep-groups; found '{runtime}'. Configure Podman to use crun."));
    }
    let crun_version = runtime_version
        .lines()
        .next()
        .unwrap_or("")
        .strip_prefix("crun version ")
        .unwrap_or("")
        .split_whitespace()
        .next()
        .unwrap_or("");
    if !version_at_least(crun_version, [1, 14, 1]) {
        return Err(format!("crun 1.14.1 or newer is required; found '{crun_version}'. Upgrade crun before running {CLI}."));
    }
    if cgroups != "v2" || manager != "systemd" {
        return Err(format!("systemd with cgroups v2 is required for rootless resource limits; found {cgroups}/{manager}. Configure cgroups v2 and Podman's systemd cgroup manager."));
    }
    if !["cpu", "memory", "pids"]
        .iter()
        .all(|required| controllers.iter().any(|controller| controller == required))
    {
        return Err("rootless CPU, memory and PID controllers must be delegated to the user. Configure systemd cgroup delegation; Podman info must list cpu, memory and pids.".into());
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

fn version_at_least(version: &str, minimum: [u32; 3]) -> bool {
    version
        .split('.')
        .take(3)
        .map(str::parse::<u32>)
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.len() == 3 && parts.as_slice() >= minimum.as_slice())
        .unwrap_or(false)
}

fn find_podman() -> Result<String, String> {
    match Command::new(PODMAN).arg("--version").output() {
        Ok(output) => {
            check_podman_output("podman --version", &output)?;
            Ok(PODMAN.into())
        }
        Err(e) if e.kind() == ErrorKind::NotFound => {
            if Command::new("docker").arg("--version").output().is_ok() {
                Err(
                    "Docker detected but Podman is required. Install Podman: https://podman.io"
                        .into(),
                )
            } else {
                Err("Podman not found. Install it: https://podman.io".into())
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
    check_podman_output("podman version", &output)?;
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        return Err("could not determine Podman version".into());
    }
    Ok(version)
}

fn podman_image_exists(executable: &str) -> Result<bool, String> {
    let output = Command::new(executable)
        .args(["image", "exists", IMAGE])
        .output()
        .map_err(|e| format!("podman image exists failed: {e}"))?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => check_podman_output("podman image exists", &output).map(|_| false),
    }
}

fn check_podman_output(operation: &str, output: &process::Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    let detail: String = String::from_utf8_lossy(&output.stderr)
        .trim()
        .chars()
        .take(4096)
        .collect();
    Err(format!(
        "{operation} failed (status {}): {}",
        output.status,
        if detail.is_empty() {
            "no error detail returned"
        } else {
            &detail
        }
    ))
}

struct PodmanHost {
    rootless: bool,
    remote: bool,
    runtime: String,
    runtime_version: String,
    cgroups: String,
    manager: String,
    controllers: Vec<String>,
}

fn podman_host(executable: &str) -> Result<PodmanHost, String> {
    // One coherent host snapshot avoids repeated expensive info collection.
    const TEMPLATE: &str = r#"{"rootless":{{json .Host.Security.Rootless}},"remote":{{json .Host.ServiceIsRemote}},"runtime":{{json .Host.OCIRuntime.Name}},"runtime_version":{{json .Host.OCIRuntime.Version}},"cgroups":{{json .Host.CgroupsVersion}},"manager":{{json .Host.CgroupManager}},"controllers":{{json .Host.CgroupControllers}}}"#;
    let output = Command::new(executable)
        .args(["info", "--format", TEMPLATE])
        .output()
        .map_err(|e| format!("podman info failed: {e}"))?;
    check_podman_output("podman info", &output)?;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "podman info returned invalid UTF-8".to_string())?;
    parse_podman_host(text)
}

fn parse_podman_host(text: &str) -> Result<PodmanHost, String> {
    if text.len() > 64 * 1024 {
        return Err("podman info response exceeds 64 KiB".into());
    }
    let JsonValue::Object(fields) =
        JsonParser::parse(text).map_err(|_| "podman info returned invalid JSON".to_string())?
    else {
        return Err("podman info returned an invalid host snapshot".into());
    };
    let field = |name: &str| {
        fields
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value)
    };
    let invalid = |name: &str| {
        format!(
            "podman info returned a missing or invalid '{name}'; check the runtime configuration"
        )
    };
    let boolean = |name: &str| match field(name) {
        Some(JsonValue::Bool(value)) => Ok(*value),
        _ => Err(invalid(name)),
    };
    let string = |name: &str| match field(name) {
        Some(JsonValue::String(value)) if !value.is_empty() => Ok(value.clone()),
        _ => Err(invalid(name)),
    };
    let controllers = match field("controllers") {
        Some(JsonValue::Array(values)) => values
            .iter()
            .map(|value| match value {
                JsonValue::String(value) if !value.is_empty() => Ok(value.clone()),
                _ => Err(invalid("controllers")),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => return Err(invalid("controllers")),
    };
    Ok(PodmanHost {
        rootless: boolean("rootless")?,
        remote: boolean("remote")?,
        runtime: string("runtime")?,
        runtime_version: string("runtime_version")?,
        cgroups: string("cgroups")?,
        manager: string("manager")?,
        controllers,
    })
}

fn podman_image_user_matches(executable: &str) -> Result<bool, String> {
    let output = Command::new(executable)
        .args(["image", "inspect", IMAGE, "--format", "{{.Config.User}}"])
        .output()
        .map_err(|e| format!("podman image inspect failed: {e}"))?;
    check_podman_output("podman image inspect", &output)?;
    let image_user = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(image_user == format!("{}:{}", current_uid(), current_gid()))
}

fn get_dockerfile_hash() -> String {
    sha256_hex(IMAGE_DOCKERFILE.as_bytes())
}

fn sha256_hex(bytes: &[u8]) -> String {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut message = bytes.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(bytes.len() as u64 * 8).to_be_bytes());
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    for chunk in message.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (i, word) in chunk.chunks_exact(4).enumerate() {
            words[i] = u32::from_be_bytes(word.try_into().unwrap());
        }
        for i in 16..64 {
            let s0 = words[i - 15].rotate_right(7)
                ^ words[i - 15].rotate_right(18)
                ^ (words[i - 15] >> 3);
            let s1 = words[i - 2].rotate_right(17)
                ^ words[i - 2].rotate_right(19)
                ^ (words[i - 2] >> 10);
            words[i] = words[i - 16]
                .wrapping_add(s0)
                .wrapping_add(words[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[i])
                .wrapping_add(words[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let temp2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }
        for (value, add) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *value = value.wrapping_add(add);
        }
    }
    state.iter().map(|word| format!("{word:08x}")).collect()
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
    check_podman_output("podman image inspect", &output)?;
    let image_hash = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(image_hash == get_dockerfile_hash())
}

fn build_image(podman: &PodmanInfo) -> Result<process::ExitStatus, String> {
    let context_dir = create_private_temp_dir("agentcage-build-")?;

    let dockerfile = context_dir.join("Dockerfile");
    fs::write(&dockerfile, IMAGE_DOCKERFILE)
        .map_err(|e| format!("cannot write temporary Dockerfile: {e}"))?;

    println!("building image {IMAGE}...");
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

fn has_terminal() -> bool {
    unsafe { libc::isatty(0) != 0 && libc::isatty(1) != 0 }
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

The current directory is mounted read-write as the project. Default login
sync also mounts an auth-only staging directory. Host home is not separately
mounted; choosing it as the project exposes it.
The container is deleted on exit. Home is ephemeral by default.
Agent login credentials are saved separately, with only auth files persisted.
Use --env NAME to keep a full persistent container home for settings too.

Requirements:
  Linux x86_64 with rootless Podman (Docker is not supported).
  Podman >= 4.9.3, crun >= 1.14.1, systemd cgroups v2 with cpu/memory/pids delegated.
  Rootless check: podman info --format '{{{{.Host.Security.Rootless}}}}' -> true

Usage:
  ac
  ac <command> [options]
  ac <agent> [--env NAME | --no-login] [-- <agent args...>]

Commands:
  init          same as ac: check runtime, build image, set up project
  codex         run Codex CLI in a disposable cage
  claude        run Claude Code in a disposable cage
  opencode      run Opencode in a disposable cage
  antigravity   run Google Antigravity CLI in a disposable cage
  shell         open a shell inside the cage
  env           list/remove persistent environments
  login         list/remove saved login credentials
  clean         remove this user's stopped, labelled AgentCage containers globally
  completions   generate completions for bash, zsh or fish
  help          show this help

Options:
  --env NAME    use full persistent environment home (.agentcage/envs/NAME/home)
  --no-login    do not sync saved agent login into this run (agent commands only)
  --            pass all following arguments unchanged to the agent
  --env and --no-login cannot be combined.

Examples:
  cd my-project &&
  ac &&
  ac codex
  ac codex --no-login
  ac codex -- --help
  ac codex -- --model MODEL
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
        "{}",
        r#"_ac_env_names() {
    local path name
    for path in .agentcage/envs/* .agentcage/envs/.[!.]* .agentcage/envs/..?*; do
        [[ -d "$path/home" && ! -L "$path" ]] || continue
        name="${path##*/}"
        [[ "$name" =~ ^[a-zA-Z0-9_.-]+$ ]] || continue
        [[ "$name" == "$cur"* ]] && COMPREPLY+=("$name")
    done
}
_ac_completions() {
    local cur="${COMP_WORDS[COMP_CWORD]}" cmd="${COMP_WORDS[1]}" i has_env=0 no_login=0 needs_env=0
    COMPREPLY=()
    for ((i=2; i<COMP_CWORD; i++)); do
        if (( needs_env )); then needs_env=0; continue; fi
        case "${COMP_WORDS[i]}" in
            --env) has_env=1; needs_env=1 ;;
            --no-login) no_login=1 ;;
            --) [[ "$cmd" != shell ]] && return ;;
        esac
    done
    if (( COMP_CWORD == 1 )); then
        COMPREPLY=($(compgen -W "init codex claude opencode antigravity shell env login remove clean help completions" -- "$cur"))
        return
    fi
    case "$cmd" in
        completions)
            (( COMP_CWORD == 2 )) && COMPREPLY=($(compgen -W "bash zsh fish" -- "$cur"))
            ;;
        env|login)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=($(compgen -W "list rm remove delete" -- "$cur"))
            elif (( COMP_CWORD == 3 )) && [[ "${COMP_WORDS[2]}" == rm || "${COMP_WORDS[2]}" == remove || "${COMP_WORDS[2]}" == delete ]]; then
                if [[ "$cmd" == login ]]; then
                    COMPREPLY=($(compgen -W "codex claude opencode antigravity" -- "$cur"))
                else
                    _ac_env_names
                fi
            fi
            ;;
        remove)
            if (( COMP_CWORD == 2 )); then
                COMPREPLY=($(compgen -W "env login" -- "$cur"))
            elif (( COMP_CWORD == 3 )); then
                case "${COMP_WORDS[2]}" in
                    env) _ac_env_names ;;
                    login) COMPREPLY=($(compgen -W "codex claude opencode antigravity" -- "$cur")) ;;
                esac
            fi
            ;;
        codex|claude|opencode|antigravity|shell)
            if (( needs_env )); then _ac_env_names; return; fi
            local options="--help"
            [[ "$cmd" != shell ]] && options+=" --"
            (( has_env == 0 && no_login == 0 )) && options+=" --env"
            [[ "$cmd" != shell ]] && (( has_env == 0 && no_login == 0 )) && options+=" --no-login"
            COMPREPLY=($(compgen -W "$options" -- "$cur"))
            ;;
    esac
    return 0
}
complete -F _ac_completions ac
"#
    );
}

fn print_completions_zsh() {
    print!(
        "{}",
        r#"# Direct source setup, including clean Zsh sessions.
if (( ! $+functions[compdef] )); then
    autoload -Uz compinit
    compinit -D
fi
_ac_env_names() {
    local path name
    local -a names
    for path in .agentcage/envs/*(ND/); do
        name="${path:t}"
        [[ -d "$path/home" && ! -L "$path" && "$name" =~ ^[a-zA-Z0-9_.-]+$ ]] && names+=("$name")
    done
    compadd -a names
}
_ac() {
    local cmd="${words[2]}" i has_env=0 no_login=0 needs_env=0
    local -a options
    for ((i=3; i<CURRENT; i++)); do
        if (( needs_env )); then needs_env=0; continue; fi
        case "${words[i]}" in
            --env) has_env=1; needs_env=1 ;;
            --no-login) no_login=1 ;;
            --) [[ "$cmd" != shell ]] && return 0 ;;
        esac
    done
    if (( CURRENT == 2 )); then
        compadd init codex claude opencode antigravity shell env login remove clean help completions
        return
    fi
    case "$cmd" in
        completions) (( CURRENT == 3 )) && compadd bash zsh fish ;;
        env|login)
            if (( CURRENT == 3 )); then
                compadd list rm remove delete
            elif (( CURRENT == 4 )) && [[ "${words[3]}" == rm || "${words[3]}" == remove || "${words[3]}" == delete ]]; then
                if [[ "$cmd" == login ]]; then compadd codex claude opencode antigravity; else _ac_env_names; fi
            fi
            ;;
        remove)
            if (( CURRENT == 3 )); then
                compadd env login
            elif (( CURRENT == 4 )); then
                case "${words[3]}" in
                    env) _ac_env_names ;;
                    login) compadd codex claude opencode antigravity ;;
                esac
            fi
            ;;
        codex|claude|opencode|antigravity|shell)
            if (( needs_env )); then _ac_env_names; return; fi
            options=(--help)
            [[ "$cmd" != shell ]] && options+=(--)
            (( has_env == 0 && no_login == 0 )) && options+=(--env)
            [[ "$cmd" != shell ]] && (( has_env == 0 && no_login == 0 )) && options+=(--no-login)
            compadd -- "${options[@]}"
            ;;
    esac
    return 0
}
compdef _ac ac
"#
    );
}

fn print_completions_fish() {
    print!(
        "{}",
        r#"complete -c ac -e
function __ac_before_separator
    __ac_wrapper_state before
end
function __ac_wrapper_state --argument-names kind
    set -l words (commandline -opc)
    set -l has_env 0
    set -l no_login 0
    set -l needs_env 0
    set -l stopped 0
    if contains -- "$words[2]" codex claude opencode antigravity shell
        for word in $words[3..-1]
            if test $needs_env -eq 1
                set needs_env 0
                continue
            end
            switch "$word"
                case --env
                    set has_env 1
                    set needs_env 1
                case --no-login
                    set no_login 1
                case --
                    if test "$words[2]" != shell
                        set stopped 1
                        break
                    end
            end
        end
    else
        test "$kind" = before
        return $status
    end
    test $stopped -eq 0; or return 1
    switch "$kind"
        case before
            return 0
        case option
            test $needs_env -eq 0
        case home
            test $needs_env -eq 0; and test $has_env -eq 0; and test $no_login -eq 0
        case value
            test $needs_env -eq 1
        case agent
            test "$words[2]" != shell
    end
end
function __ac_at
    set -l words (commandline -opc)
    test (count $words) -eq $argv[1]; or return 1
    if test (count $argv) -ge 2
        test "$words[2]" = "$argv[2]"; or return 1
    end
    if test (count $argv) -ge 3
        contains -- "$words[3]" $argv[3..-1]; or return 1
    end
end
function __ac_env_names
    for path in .agentcage/envs/* .agentcage/envs/.*
        if test -d "$path/home"; and not test -L "$path"
            set -l name (string replace -r '.*/' '' -- "$path")
            contains -- "$name" . ..; and continue
            string match -qr '^[a-zA-Z0-9_.-]+$' -- "$name"; and printf '%s\n' "$name"
        end
    end
end
function __ac_wrapper_option
    __ac_wrapper_state option
end
function __ac_can_choose_home
    __ac_wrapper_state home
end
function __ac_env_value
    __ac_wrapper_state value
end

complete -c ac -f
complete -c ac -n '__ac_at 1' -a 'init codex claude opencode antigravity shell env login remove clean help completions'
complete -c ac -n '__ac_before_separator; and __ac_at 2 env' -a 'list rm remove delete'
complete -c ac -n '__ac_before_separator; and __ac_at 2 login' -a 'list rm remove delete'
complete -c ac -n '__ac_before_separator; and __ac_at 2 remove' -a 'env login'
complete -c ac -n '__ac_before_separator; and __ac_at 2 completions' -a 'bash zsh fish'
complete -c ac -n '__ac_before_separator; and __ac_at 3 login rm remove delete' -a 'codex claude opencode antigravity'
complete -c ac -n '__ac_before_separator; and __ac_at 3 remove login' -a 'codex claude opencode antigravity'
complete -c ac -n '__ac_before_separator; and __ac_at 3 env rm remove delete' -a '(__ac_env_names)'
complete -c ac -n '__ac_before_separator; and __ac_at 3 remove env' -a '(__ac_env_names)'
complete -c ac -n '__ac_can_choose_home' -l env -r -a '(__ac_env_names)' -d 'persistent environment name'
complete -c ac -n '__ac_env_value' -a '(__ac_env_names)'
complete -c ac -n '__ac_can_choose_home; and __ac_wrapper_state agent' -l no-login -d 'disable saved login'
# Literal candidate also works when Fish treats a consumed --env value of --
# as its own end-of-options marker.
complete -c ac -n '__ac_wrapper_option' -a '--help' -d 'wrapper help'
complete -c ac -n '__ac_wrapper_option; and __ac_wrapper_state agent' -a '--' -d 'pass following arguments to the agent'
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
        let (argv, _) = build_run_argv(
            &cwd,
            "codex",
            &[],
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(argv.iter().any(|arg| arg == "--network=host"));
        assert!(!argv.iter().any(|arg| arg == "-p"));
        assert!(!argv
            .iter()
            .any(|arg| arg.to_string_lossy().contains("1455")));
    }

    #[test]
    fn runs_hide_project_agentcage_state_with_tmpfs() {
        let cwd = env::current_dir().unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            "codex",
            &[],
            &HomeMount::Disposable,
            &LoginMount::None,
        )
        .unwrap();

        assert!(argv.iter().any(|arg| arg == format!("type=tmpfs,destination={CONTAINER_CAGE_DIR},tmpfs-size=64M,tmpfs-mode=0700,U=true,notmpcopyup").as_str()));
    }

    #[test]
    fn no_login_runs_without_login_sync_mount_or_wrapper() {
        let cwd = env::current_dir().unwrap();
        let (argv, _) = build_run_argv(
            &cwd,
            "codex",
            &[],
            &HomeMount::Disposable,
            &LoginMount::Disabled,
        )
        .unwrap();

        assert!(!argv
            .iter()
            .any(|arg| arg.to_string_lossy().contains(CONTAINER_LOGIN_SYNC)));
        assert!(!argv.iter().any(|arg| arg == "agentcage-login-sync"));
        assert!(argv.iter().any(|arg| arg == "codex"));
    }

    #[test]
    fn rejects_unknown_global_options() {
        let err = parse_err(&["--bogus"]);
        assert!(err.contains("unknown global option"));
    }

    #[test]
    fn agent_metadata_matches_parser_and_login_paths() {
        for (name, binary) in agent::SUPPORTED_AGENTS {
            assert_eq!(agent::resolve(name).unwrap(), *binary);
            let parsed = parse(&[name.to_string(), "--".into(), "--help".into()]).unwrap();
            assert!(
                matches!(parsed, Cmd::Agent { name: actual, args, .. } if actual == *name && args == ["--help"])
            );
            assert!(!login_paths_for_agent(name).is_empty());
            for path in login_paths_for_agent(name) {
                assert!(Path::new(path)
                    .components()
                    .all(|component| matches!(component, Component::Normal(_))));
            }
        }
        assert!(parse_err(&["codex", "--env", "one", "--env", "two"]).contains("duplicate"));
        assert!(parse_err(&["codex", "--env", "one", "--no-login"]).contains("cannot be combined"));
        assert!(
            parse_err(&["codex", "--model", "example"]).contains("pass agent options after '--'")
        );
    }

    #[test]
    fn content_hash_matches_sha256_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn credential_json_checks_structure_whitespace_and_escapes() {
        let rel = ".local/share/opencode/account.json";
        for content in [
            r#"{"accounts":[],"active_account_id":null,"active_org_id":null}"#,
            r#"{"accounts":[ ],"active_account_id":null,"active_org_id":null}"#,
        ] {
            assert!(
                !should_persist_login_bytes("opencode", rel, content.as_bytes()).unwrap(),
                "{content}"
            );
        }
        for content in [
            r#"{"accounts":[{"id":"test","email":"fixture@invalid.example","url":"https://example.invalid","access_token":"fixture-access","refresh_token":"fixture-refresh","token_expiry":null}],"active_account_id":null,"active_org_id":null}"#,
            r#"{ "accounts" : [ { "id" : "has space and \" quote", "email":"fixture@invalid.example", "url":"https://example.invalid", "access_token":"fixture-access", "refresh_token":"fixture-refresh", "token_expiry":0 } ], "active_account_id":null,"active_org_id":null }"#,
            r#"{"\u0061ccounts":[{"id":"\uD83D\uDE00","email":"fixture@invalid.example","url":"https://example.invalid","access_token":"fixture-access","refresh_token":"fixture-refresh","token_expiry":9223372036854775807}],"active_account_id":"\uD83D\uDE00","active_org_id":"fixture-org"}"#,
        ] {
            assert!(
                should_persist_login_bytes("opencode", rel, content.as_bytes()).unwrap(),
                "{content}"
            );
        }
        for content in [
            r#"{"accounts":[]}"#,
            r#"{"nested":{"accounts":[{}]}}"#,
            r#"{"accounts":[{}],"active_account_id":null,"active_org_id":null}"#,
            r#"{"accounts":{ }}"#,
            r#"{"accounts":false}"#,
            r#"{"accounts":null}"#,
            r#"{"accounts":"token"}"#,
            r#"{"accounts":[42]}"#,
            "false",
            "null",
            "[]",
            r#"{"accounts":[}"#,
            r#"{"accounts":[],"accounts":[{}]}"#,
            r#"{"accounts":["\uD800"]}"#,
            r#"{"accounts":["\u+061"]}"#,
            r#"{"accounts":[01]}"#,
            r#"{"accounts":[1.]}"#,
            "{} trailing",
        ] {
            assert!(
                should_persist_login_bytes("opencode", rel, content.as_bytes()).is_err(),
                "{content}"
            );
        }
        assert!(should_persist_login_bytes(
            "claude",
            ".claude.json",
            br#"{"primaryApiKey":"has space"}"#
        )
        .unwrap());
        assert!(should_persist_login_bytes(
            "claude",
            ".claude.json",
            br#"{"primaryApiKey":false}"#
        )
        .is_err());
        assert!(should_persist_login_bytes("codex", "auth.json", &[0xff]).is_err());
        assert!(should_persist_login_bytes("antigravity", "token", &[0xff]).unwrap());
        assert!(should_persist_login_bytes("claude", ".claude.json", &[0xff]).is_err());
        assert!(should_persist_login_bytes(
            "claude",
            ".claude.json",
            br#"{"primaryApiKey":"fixture\u+061"}"#
        )
        .is_err());
    }

    #[test]
    fn pinned_codex_auth_variants_validate_without_provider_requests() {
        for content in [
            r#"{"auth_mode":"apikey","OPENAI_API_KEY":"fixture"}"#,
            r#"{"tokens":{"id_token":"e30.e30.fixture","access_token":"fixture","refresh_token":"fixture","account_id":null},"last_refresh":"2024-02-29T23:59:59.123Z"}"#,
            r#"{"auth_mode":"personalAccessToken","personal_access_token":"fixture","last_refresh":"2026-10-04T00:00:00+02:00"}"#,
            r#"{"agent_identity":"fixture"}"#,
            r#"{"agent_identity":{"agent_runtime_id":"fixture","agent_private_key":"fixture","account_id":"fixture","chatgpt_user_id":"fixture","email":null,"plan_type":"future-plan","chatgpt_account_is_fedramp":false}}"#,
            r#"{"bedrock_api_key":{"api_key":"fixture","region":"us-east-1"}}"#,
            r#"{"bedrock_access_keys":{"access_key_id":"fixture","secret_access_key":"fixture","session_token":null}}"#,
        ] {
            assert_eq!(
                should_persist_login_bytes("codex", "auth.json", content.as_bytes()),
                Ok(true),
                "{content}"
            );
        }
        for content in [r#"{}"#, r#"{"OPENAI_API_KEY":null,"tokens":null}"#] {
            assert_eq!(
                should_persist_login_bytes("codex", "auth.json", content.as_bytes()),
                Ok(false)
            );
        }
        for content in [
            r#"{"OPENAI_API_KEY":42}"#,
            r#"{"tokens":{"id_token":"e30.ZmFsc2U.fixture","access_token":"fixture","refresh_token":"fixture"}}"#,
            r#"{"tokens":{"id_token":"e30.eyJlbWFpbCI6NDJ9.fixture","access_token":"fixture","refresh_token":"fixture"}}"#,
            r#"{"tokens":{"id_token":"e30.e31.fixture","access_token":"fixture","refresh_token":"fixture"}}"#,
            r#"{"OPENAI_API_KEY":"fixture","last_refresh":"2023-02-29T00:00:00Z"}"#,
            r#"{"OPENAI_API_KEY":"fixture","last_refresh":"2026-10-04T25:00:00Z"}"#,
            r#"{"bedrock_access_keys":{"access_key_id":"fixture","secret_access_key":false}}"#,
            r#"{"agent_identity":{"agent_runtime_id":"fixture","agent_private_key":"fixture"}}"#,
        ] {
            assert!(
                should_persist_login_bytes("codex", "auth.json", content.as_bytes()).is_err(),
                "{content}"
            );
        }
    }

    #[test]
    fn positional_agent_arguments_survive_a_later_separator() {
        for args in [
            vec!["codex", "exec", "--", "-"],
            vec!["codex", "--no-login", "exec", "--", "-"],
        ] {
            let args: Vec<_> = args.into_iter().map(String::from).collect();
            let Cmd::Agent { args, .. } = parse(&args).unwrap() else {
                panic!("expected agent command");
            };
            assert_eq!(args, ["exec", "-"]);
        }
    }

    #[test]
    fn opaque_agent_arguments_survive_without_utf8_conversion() {
        use std::os::unix::ffi::OsStringExt;
        let opaque = OsString::from_vec(vec![b'f', 0xff]);
        let parsed = parse_os(&[
            "codex".into(),
            "--env".into(),
            "--".into(),
            "exec".into(),
            "--".into(),
            opaque.clone(),
        ])
        .unwrap();
        let Cmd::Agent { env, args, .. } = parsed else {
            panic!("expected agent");
        };
        assert_eq!(env.as_deref(), Some("--"));
        assert_eq!(args, [OsString::from("exec"), opaque.clone()]);
        assert!(parse_os(&["codex".into(), opaque]).is_err());
    }

    #[test]
    fn podman_host_snapshot_preserves_boolean_values_and_rejects_wrong_types() {
        let json = r#"{"rootless":true,"remote":false,"runtime":"crun","runtime_version":"crun version 1.14.1\ncommit: fixture","cgroups":"v2","manager":"systemd","controllers":["cpu","memory","pids"]}"#;
        let host = parse_podman_host(json).unwrap();
        assert!(host.rootless);
        assert!(!host.remote);
        assert_eq!(host.controllers, ["cpu", "memory", "pids"]);
        assert!(host.runtime_version.contains('\n'));
        for bad in [
            json.replace("true", "\"true\""),
            json.replace("[\"cpu\",\"memory\",\"pids\"]", "\"[cpu memory pids]\""),
            json.replace("false", "null"),
            "{}".to_string(),
        ] {
            assert!(parse_podman_host(&bad).is_err());
        }
    }

    #[test]
    fn anchored_credentials_reject_symlinks_special_files_and_preserve_bytes() {
        let root = temp_test_root("anchored-credentials");
        let outside = temp_test_root("anchored-outside");
        create_private_dir_all(&root).unwrap();
        create_private_dir_all(&outside).unwrap();
        let directory = open_private_root(&root).unwrap();
        let sentinel = outside.join("auth.json");
        fs::write(&sentinel, "outside-fixture").unwrap();
        fs::set_permissions(&sentinel, fs::Permissions::from_mode(0o644)).unwrap();
        write_credential(&directory, "auth.json", &[0xff, b'\n']).unwrap();
        assert_eq!(
            read_credential(&directory, "auth.json").unwrap().unwrap(),
            [0xff, b'\n']
        );
        assert_eq!(mode(&root.join("auth.json")), 0o600);
        for rel in login_paths_for_agent("opencode")
            .iter()
            .chain(login_paths_for_agent("antigravity"))
        {
            let first = Path::new(rel).components().next().unwrap().as_os_str();
            let link = root.join(first);
            std::os::unix::fs::symlink(&outside, &link).unwrap();
            assert!(read_credential(&directory, rel).is_err());
            fs::remove_file(&link).unwrap();
        }
        for parent in [".local", ".local/share", ".local/share/opencode"] {
            write_credential(&directory, ".local/share/opencode/auth.json", b"{}").unwrap();
            fs::remove_dir_all(root.join(parent)).unwrap();
            std::os::unix::fs::symlink(&outside, root.join(parent)).unwrap();
            assert!(read_credential(&directory, ".local/share/opencode/auth.json").is_err());
            fs::remove_file(root.join(parent)).unwrap();
        }
        std::os::unix::fs::symlink(&outside, root.join(".codex")).unwrap();
        assert!(read_credential(&directory, ".codex/auth.json").is_err());
        assert!(harden_known_login_files_in_home(&root).is_err());
        assert_eq!(mode(&sentinel), 0o644);
        fs::remove_file(root.join(".codex")).unwrap();
        std::os::unix::fs::symlink(&sentinel, root.join("linked.json")).unwrap();
        assert!(read_credential(&directory, "linked.json").is_err());
        fs::hard_link(&sentinel, root.join("hard.json")).unwrap();
        assert!(read_credential(&directory, "hard.json").is_err());
        let fifo = CString::new(root.join("fifo").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert!(read_credential(&directory, "fifo").is_err());
        assert_eq!(fs::read_to_string(sentinel).unwrap(), "outside-fixture");
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn private_directories_and_removal_stay_anchored_when_parents_change() {
        let root = temp_test_root("state-parent");
        let outside = temp_test_root("state-outside");
        create_private_dir_all(&root).unwrap();
        create_private_dir_all(&outside.join("fixture-env")).unwrap();
        fs::write(outside.join("fixture-env/sentinel"), "outside-fixture").unwrap();
        create_private_dir_all(&root.join("envs/fixture-env/home")).unwrap();
        fs::write(root.join("envs/fixture-env/home/sentinel"), "owned-fixture").unwrap();
        let dir = open_private_root(&root).unwrap();
        let (_parent, anchored) = directory_for_removal(&dir, "envs/fixture-env")
            .unwrap()
            .unwrap();
        fs::rename(root.join("envs"), root.join("original-envs")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("envs")).unwrap();
        assert!(directory_for_removal(&dir, "envs/fixture-env").is_err());
        assert!(create_private_dir_all(&root.join("envs/new/private")).is_err());
        assert!(!outside.join("new").exists());
        assert!(open_private_root(&root.join("envs/fixture-env")).is_err());
        fs::remove_dir_all(anchored).unwrap();
        assert!(!root.join("original-envs/fixture-env").exists());
        assert_eq!(
            fs::read_to_string(outside.join("fixture-env/sentinel")).unwrap(),
            "outside-fixture"
        );
        fs::remove_dir_all(root).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn anchored_file_survives_path_replacement_and_enforces_size_on_read() {
        let root = temp_test_root("credential-replacement");
        create_private_dir_all(&root).unwrap();
        let dir = open_private_root(&root).unwrap();
        write_credential(&dir, "auth.json", b"original").unwrap();
        let mut opened = open_credential(&dir, "auth.json").unwrap().unwrap();
        fs::rename(root.join("auth.json"), root.join("old.json")).unwrap();
        write_credential(&dir, "auth.json", b"replacement").unwrap();
        let mut content = Vec::new();
        opened.read_to_end(&mut content).unwrap();
        assert_eq!(content, b"original");
        write_credential(&dir, "copy.json", &content).unwrap();
        assert_eq!(
            read_credential(&dir, "copy.json").unwrap().unwrap(),
            b"original"
        );
        let large = fs::File::create(root.join("large.json")).unwrap();
        large.set_len(MAX_LOGIN_FILE_BYTES).unwrap();
        assert_eq!(
            read_credential(&dir, "large.json").unwrap().unwrap().len() as u64,
            MAX_LOGIN_FILE_BYTES
        );
        let opened_before_growth = open_credential(&dir, "large.json").unwrap().unwrap();
        large.set_len(MAX_LOGIN_FILE_BYTES + 1).unwrap();
        assert!(read_open_credential(opened_before_growth, "large.json").is_err());
        assert!(read_credential(&dir, "large.json").is_err());
        fs::remove_dir_all(root).unwrap();
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
                assert_eq!(args, vec![OsString::from("--help")]);
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
    fn stale_login_staging_cleanup_keeps_unsafe_owner_files_without_reading_them() {
        let root = temp_test_root("unsafe-owner");
        create_private_dir_all(&root).unwrap();
        let outside = root.join("outside-marker");
        fs::write(&outside, "fixture, not a PID").unwrap();
        for name in ["fifo", "symlink", "oversize"] {
            let staging = root.join(format!("{LOGIN_STAGING_PREFIX}codex-{name}"));
            create_private_dir_all(&staging).unwrap();
            let marker = staging.join(LOGIN_STAGING_OWNER_FILE);
            match name {
                "fifo" => {
                    let path = CString::new(marker.as_os_str().as_bytes()).unwrap();
                    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
                }
                "symlink" => std::os::unix::fs::symlink(&outside, marker).unwrap(),
                _ => fs::write(marker, "1".repeat(33)).unwrap(),
            }
            assert!(login_staging_owner_alive(&staging));
        }
        assert_eq!(
            cleanup_stale_login_staging_in(&root, "codex", Duration::ZERO).unwrap(),
            0
        );
        assert_eq!(fs::read_to_string(&outside).unwrap(), "fixture, not a PID");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn antigravity_maps_to_agy_and_persists_only_token_file() {
        assert_eq!(agent::resolve("antigravity").unwrap(), "agy");
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

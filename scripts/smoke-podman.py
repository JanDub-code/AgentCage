#!/usr/bin/env python3
"""Opt-in real rootless containment/agent smoke; exit 77 is explicitly unverified."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import selectors
import shlex
import shutil
import signal
import socket
import subprocess
import tempfile
import time
import uuid


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", required=True, type=Path)
parser.add_argument("--run", action="store_true", help="authorize disposable labelled real containers")
parser.add_argument("--required", action="store_true", help="missing/unavailable runtime fails rather than exits 77")
parser.add_argument("--storage-root", type=Path, help="isolated Podman test store (also needs --storage-runroot)")
parser.add_argument("--storage-runroot", type=Path, help="isolated Podman test runroot")
args = parser.parse_args()


def unavailable(reason):
    print("UNVERIFIED: " + reason, flush=True)
    raise SystemExit(1 if args.required else 77)


if not args.run:
    unavailable("real smoke is opt-in; invoke --run (release checks also require --required)")
podman = shutil.which("podman")
if os.name != "posix" or not podman or not args.binary.is_file():
    unavailable("Linux, Podman and a built ac binary are required")
binary = args.binary.resolve()
if bool(args.storage_root) != bool(args.storage_runroot):
    parser.error("--storage-root and --storage-runroot must be supplied together")
podman_options = (["--root", str(args.storage_root.resolve()), "--runroot", str(args.storage_runroot.resolve()),
                   "--storage-driver", "overlay"] if args.storage_root else [])
host_env = {name: os.environ[name] for name in ("PATH", "HOME", "USER", "LOGNAME", "LANG", "XDG_RUNTIME_DIR",
            "DBUS_SESSION_BUS_ADDRESS", "XDG_CONFIG_HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME") if name in os.environ}
info = subprocess.run([podman, *podman_options, "info", "--format", "json"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=30)
if info.returncode:
    unavailable("Podman info failed: " + info.stderr.decode(errors="replace"))
runtime = json.loads(info.stdout)
print(json.dumps({"actual_runtime": runtime.get("host"), "podman": runtime.get("version")}, sort_keys=True), flush=True)
if not runtime.get("host", {}).get("security", {}).get("rootless"):
    unavailable("Podman is not rootless")
label = "io.agentcage.smoke=" + uuid.uuid4().hex
passed = []


def raw(arguments, timeout=30):
    return subprocess.run([podman, *podman_options, *arguments], env=host_env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)


def owned_ids():
    result = raw(["ps", "-aq", "--no-trunc", "--filter", "label=" + label])
    assert result.returncode == 0, result.stderr
    return result.stdout.decode().split()


def read_until(fd, marker, timeout=30):
    output = b""
    deadline = time.monotonic() + timeout
    with selectors.DefaultSelector() as selector:
        selector.register(fd, selectors.EVENT_READ)
        while marker not in output:
            remaining = deadline - time.monotonic()
            assert remaining > 0 and selector.select(remaining), repr(output)
            output += os.read(fd, 65536)
    return output


with tempfile.TemporaryDirectory(prefix="ac-live-") as temp:
    root = Path(temp)
    project = root / "project with spaces"
    project.mkdir()
    home = root / "host-home"
    home.mkdir(mode=0o700)
    (home / ".ssh").mkdir(mode=0o700)
    sentinel = home / ".ssh/fixture-private"
    sentinel.write_text("fixture host credential sentinel")
    cage = project / ".agentcage"
    cage.mkdir(mode=0o700)
    (cage / "fixture-sentinel").write_text("fixture cage state sentinel")
    (project / "host-home-link").symlink_to(sentinel)
    temporary = root / "tmp"
    temporary.mkdir(mode=0o700)
    tools = root / "bin"
    tools.mkdir()
    # Freeze the compiled artifact for the full smoke, even while a colleague
    # is producing the next build in another target directory.
    frozen_binary = tools / "ac-under-test"
    shutil.copy2(binary, frozen_binary)
    binary = frozen_binary
    binary_sha = hashlib.sha256(binary.read_bytes()).hexdigest()
    # Restore only the Podman process environment, so an isolated ac HOME can use
    # an existing built image. All ac login/home fixtures remain private.
    wrapper = tools / "podman"
    wrapper.write_text("#!/usr/bin/env python3\nimport json, os, socket, sys\n" +
                       "args=sys.argv[1:]\nenv=" + repr(host_env) + "\n" +
                       "if args and args[0]=='ps' and os.environ.get('AC_SMOKE_CLEAN_SCOPE'):\n" +
                       "    args+=['--filter', 'label='+" + repr(label) + "]\n" +
                       "if args and args[0]=='rm' and os.environ.get('AC_SMOKE_RACE_SOCKET') and os.environ['AC_SMOKE_RACE_TARGET'] in args:\n" +
                       "    with socket.socket(socket.AF_UNIX) as channel:\n" +
                       "        channel.connect(os.environ['AC_SMOKE_RACE_SOCKET'])\n" +
                       "        channel.sendall((json.dumps({'target':os.environ['AC_SMOKE_RACE_TARGET']})+'\\n').encode())\n" +
                       "        with channel.makefile('rb') as stream: stream.readline()\n" +
                       "if args and args[0]=='run':\n" +
                       "    args[1:1]=['--label'," + repr(label) + "]\n" +
                       "    if os.environ.get('AC_SMOKE_ARGV_PROBE'):\n" +
                       "        args[1:1]=['--env','PATH=/workspace/project/argv-probe:/opt/agent-tools/bin:/usr/local/bin:/usr/bin:/bin']\n" +
                       "    if os.environ.get('AC_SMOKE_SMALL_LIMITS'):\n" +
                       "        replacements={'--cpus=2':'--cpus=0.5','--memory=4g':'--memory=128m','--pids-limit=256':'--pids-limit=32'}\n" +
                       "        args=[replacements.get(arg,arg) for arg in args]\n" +
                       "os.execve(" + repr(podman) + ",[" + repr(podman) + "]+" + repr(podman_options) + "+args,env)\n")
    wrapper.chmod(0o700)
    env = {"PATH": str(tools) + ":/usr/bin:/bin", "HOME": str(home), "LANG": "C.UTF-8", "TMPDIR": str(temporary),
           "XDG_DATA_HOME": str(home / "data"), "XDG_CONFIG_HOME": str(home / "config"),
           "XDG_STATE_HOME": str(home / "state"), "XDG_CACHE_HOME": str(home / "cache")}

    def cli(arguments, script=None, small=False, timeout=60, extra=None, expect_teardown=True):
        result = subprocess.run([str(binary), *arguments], cwd=project, env=env | (extra or {}) | ({"AC_SMOKE_SMALL_LIMITS": "1"} if small else {}),
                                input=script if isinstance(script, bytes) else script.encode() if script else b"",
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)
        if expect_teardown:
            assert not owned_ids(), "container remained after --rm: " + repr(owned_ids())
        return result

    def shell(script, small=False):
        result = cli(["shell"], "set -eu\n" + script + "\n", small)
        assert result.returncode == 0, result.stderr.decode(errors="replace")
        return result.stdout

    try:
        check = shell("python3 - <<'PY'\n" +
                      "import errno,json,os,pathlib,stat\n" +
                      "home=pathlib.Path(os.environ['HOME'])\n" +
                      "assert os.getuid()==" + str(os.getuid()) + " and os.getgid()==" + str(os.getgid()) + "\n" +
                      "assert stat.S_IMODE(home.stat().st_mode)==0o700\n" +
                      "assert not pathlib.Path('/workspace/project/.agentcage/fixture-sentinel').exists()\n" +
                      "assert not pathlib.Path(" + repr(str(sentinel)) + ").exists()\n" +
                      "assert not pathlib.Path('/workspace/project/host-home-link').exists()\n" +
                      "assert not pathlib.Path('/workspace/login-sync').exists()\n" +
                      "assert not (home/'fixture-disposable').exists()\n" +
                      "(home/'fixture-disposable').write_text('fixture')\n" +
                      "pathlib.Path('/workspace/project/fixture-project-write').write_text('fixture project')\n" +
                      "pathlib.Path('/workspace/project/.agentcage/fixture-overlay').write_text('fixture overlay')\n" +
                      "try: pathlib.Path('/usr/fixture-readonly').write_text('fixture')\n" +
                      "except OSError as error: assert error.errno==errno.EROFS,error\n" +
                      "else: raise AssertionError('image root is writable')\n" +
                      "cg=pathlib.Path('/sys/fs/cgroup')\n" +
                      "cpu=list(map(int,(cg/'cpu.max').read_text().split()))\n" +
                      "assert cpu[0]/cpu[1]==2\n" +
                      "assert (cg/'memory.max').read_text().strip()==str(4*1024**3)\n" +
                      "assert (cg/'pids.max').read_text().strip()=='256'\n" +
                      "print(json.dumps({'uid':os.getuid(),'gid':os.getgid(),'default_cpu':cpu,'default_memory':(cg/'memory.max').read_text().strip(),'default_pids':256}))\nPY")
        print(check.decode(), end="", flush=True)
        assert (cage / "fixture-sentinel").read_text() == "fixture cage state sentinel"
        assert not (cage / "fixture-overlay").exists()
        assert sentinel.read_text() == "fixture host credential sentinel"
        assert (project / "fixture-project-write").read_text() == "fixture project"
        assert cage.stat().st_mode & 0o777 == 0o700
        assert shell("test ! -e \"$HOME/fixture-disposable\"\nprintf 'fixture exact pipe\\n'") == b"fixture exact pipe\n"
        passed.append("actual UID/GID/private home, cage/home sentinels, RO root, disposable home, writable project, exact stdout, default limits and teardown")
        pids = shell("python3 - <<'PY'\nimport errno,os,pathlib,signal\nchildren=[]\n" +
                     "try:\n for i in range(64):\n  try: pid=os.fork()\n  except OSError as error:\n   assert error.errno==errno.EAGAIN,error\n   break\n  if pid==0:\n   signal.pause();os._exit(0)\n  children.append(pid)\n else: raise AssertionError('PID limit not enforced')\n" +
                     " assert int(pathlib.Path('/sys/fs/cgroup/pids.events').read_text().split()[1])>0\n" +
                     " print('fixture PID limit enforced',len(children))\n" +
                     "finally:\n for pid in children: os.kill(pid,signal.SIGKILL)\n for pid in children: os.waitpid(pid,0)\nPY", small=True)
        print(pids.decode(), end="", flush=True)
        cpu = shell("python3 - <<'PY'\nimport os,pathlib,time\n" +
                    "def throttled():\n return dict(line.split() for line in pathlib.Path('/sys/fs/cgroup/cpu.stat').read_text().splitlines())['nr_throttled']\n" +
                    "before=int(throttled());children=[]\n" +
                    "for i in range(3):\n pid=os.fork()\n if pid==0:\n  stop=time.monotonic()+2\n  while time.monotonic()<stop: pass\n  os._exit(0)\n children.append(pid)\n" +
                    "for pid in children: assert os.waitpid(pid,0)[1]==0\n" +
                    "after=int(throttled());assert after>before,(before,after)\nprint('fixture CPU quota throttled',after-before)\nPY", small=True)
        print(cpu.decode(), end="", flush=True)
        oom = cli(["shell"], "python3 -c 'x=bytearray(256*1024*1024); print(\"memory limit failed\")'\n", small=True)
        assert oom.returncode == 137 and b"memory limit failed" not in oom.stdout, (oom.returncode, oom.stdout, oom.stderr)
        passed.append("bounded 0.5 CPU/128 MiB/32 PID fixture enforcement (throttling, OOM137, EAGAIN), with default limits verified separately")
        for name in ("codex", "claude", "opencode", "antigravity"):
            for option in ("--help", "--version"):
                result = cli([name, "--no-login", "--", option], timeout=120)
                assert result.returncode == 0, (name, option, result.returncode, result.stderr.decode(errors="replace"))
                content = (result.stdout + result.stderr).decode(errors="replace")
                if option == "--version":
                    assert re.search(r"\d+\.\d+\.\d+", content), (name, option, "agent version missing")
                else:
                    assert ("agy" if name == "antigravity" else name) in content.lower(), (name, option, "agent help missing")
                print(name + " " + option + ": " + (result.stdout or result.stderr).decode(errors="replace").splitlines()[0], flush=True)
            passed.append(name + " help/version starts in final read-only runtime")
        # Only the project-local agent fixture is replaced. Podman, the OCI
        # argument transport, embedded launcher, login wrapper and finalizer
        # remain the real production path, with the original argument bytes.
        probe = project / "argv-probe"
        probe.mkdir()
        agent_probe = """#!/usr/bin/python3
import errno,json,os,pathlib,sys
argv=[os.fsencode(arg) for arg in sys.argv[1:]]
if argv and argv[0]==b'write-invalid':
    home=pathlib.Path(os.environ['HOME'])
    if pathlib.Path(sys.argv[0]).name=='claude':
        (home/'.claude/.credentials.json').write_bytes(argv[1])
        (home/'.claude.json').write_bytes(b'{"primaryApiKey":"fixture-new"}')
    else:
        (home/'auth.json').write_bytes(argv[1])
        (home/'.codex/auth.json').write_bytes(b'{"OPENAI_API_KEY":"fixture-new"}')
elif argv and argv[0]==b'export-delete-fault':
    home=pathlib.Path(os.environ['HOME'])
    (home/'auth.json').unlink()
    (home/'.codex/auth.json').unlink()
    stage=pathlib.Path('/workspace/login-sync')
    (stage/'.codex').chmod(0o500)
    try:
        (stage/'.codex/auth.json').unlink()
    except OSError as error:
        assert error.errno==errno.EACCES,error
    else:
        raise AssertionError('fixture did not enforce EACCES')
print(json.dumps({'argv_hex':[arg.hex() for arg in argv],'stdin_hex':sys.stdin.buffer.read().hex()}))
sys.exit(42 if b'exit42' in argv else 0)
"""
        for name in ("codex", "claude"):
            (probe / name).write_text(agent_probe)
            (probe / name).chmod(0o755)
        auth_root = home / "data/agentcage/logins"
        codex_auth = b'{"OPENAI_API_KEY":"fixture-original"}'
        oauth_auth = b'{"claudeAiOauth":{"accessToken":"fixture-access","refreshToken":"fixture-refresh","expiresAt":1900000000000}}'
        api_auth = b'{"primaryApiKey":"fixture-original"}'
        saved_paths = {"codex": [("auth.json", codex_auth), (".codex/auth.json", codex_auth)],
                       "claude": [(".claude/.credentials.json", oauth_auth), (".claude.json", api_auth)]}
        for name, files in saved_paths.items():
            for relative, content in files:
                destination = auth_root / name / relative
                destination.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
                destination.write_bytes(content)
                destination.chmod(0o600)
        stdin = b"fixture stdin\x00\xff\n"
        for login in (False, True):
            for status in (0, 42):
                tail = [b"exit42" if status else b"exit0", b"two words", b"opaque-\xff-\xfe", b"", b"--",
                        "žluťoučký".encode(), b"x" * (70 * 1024) + b"\xff"]
                result = cli(["codex", *([] if login else ["--no-login"]), "--", *tail], stdin,
                             extra={"AC_SMOKE_ARGV_PROBE": "1"})
                assert result.returncode == status, (login, status, result.returncode, result.stderr)
                observed = json.loads(result.stdout)
                assert observed == {"argv_hex": [arg.hex() for arg in tail], "stdin_hex": stdin.hex()}, (login, status, "opaque argv/stdin changed")
                assert all((auth_root / "codex" / relative).read_bytes() == content for relative, content in saved_paths["codex"])
                assert not list(temporary.glob("agentcage-login-*")), "staging remained after argv probe"
        passed.append("actual Podman/OCI opaque argv bytes, empty/Unicode/70KiB arguments and binary stdin, login/no-login with exact exits 0/42")
        for name, invalid_documents in {
            "codex": [b'{bad-json', b'{"tokens":42}'],
            "claude": [b'{"claudeAiOauth":{"accessToken":false,"refreshToken":42}}',
                       b'{"claudeAiOauth":{"unrelatedSetting":true}}'],
        }.items():
            for invalid in invalid_documents:
                for status in (0, 42):
                    result = cli([name, "--", "write-invalid", invalid.decode(), "exit42" if status else "exit0"],
                                 extra={"AC_SMOKE_ARGV_PROBE": "1"})
                    assert result.returncode == (42 if status else 1), (name, status, result.returncode, result.stderr)
                    assert b"invalid" in result.stderr.lower(), result.stderr
                    assert all((auth_root / name / relative).read_bytes() == content for relative, content in saved_paths[name]), (name, "partial or invalid credential snapshot published")
                    assert not list(temporary.glob("agentcage-login-*")), "invalid staging was retained"
        passed.append("actual embedded login wrapper rejects malformed Codex/Claude auth and preserves the entire saved snapshot, exits 1/42")
        for status in (0, 42):
            result = cli(["codex", "--", "export-delete-fault", "exit42" if status else "exit0"],
                         extra={"AC_SMOKE_ARGV_PROBE": "1"})
            assert result.returncode == (42 if status else 1), (status, result.returncode, result.stderr)
            assert b"auth file sync failed" in result.stderr and b"failed to save login" in result.stderr, result.stderr
            assert all((auth_root / "codex" / relative).read_bytes() == content for relative, content in saved_paths["codex"]), "failed export published partial logout"
            # The fixture intentionally made a staging parent read-only; a
            # truthful cleanup error must identify any remaining private copy.
            stages = list(temporary.glob("agentcage-login-codex-*"))
            for stage in stages:
                assert b"cannot remove login staging " + os.fsencode(stage) in result.stderr, result.stderr
                nested = stage / ".codex"
                assert not nested.is_symlink()
                nested.chmod(0o700)
                shutil.rmtree(stage)
        passed.append("actual kernel EACCES during the second copy-out deletion preserves the whole accepted snapshot and exits 1/42")
        # The native API-key login writer/status reader use local storage;
        # this synthetic value is never evidence of provider authentication.
        native_key = b"fixture-native-local-api-key"
        native_login = cli(["codex", "--", "login", "--with-api-key"], native_key + b"\n", timeout=120)
        assert native_login.returncode == 0, native_login.stderr
        native_auth = json.loads((auth_root / "codex/auth.json").read_bytes())
        assert native_auth["OPENAI_API_KEY"] == native_key.decode(), "native Codex credential was not persisted"
        native_status = cli(["codex", "--", "login", "status"], timeout=120)
        assert native_status.returncode == 0 and b"Logged in using an API key" in native_status.stderr, native_status.stderr
        assert (auth_root / "codex/.codex/auth.json").read_bytes() == codex_auth
        assert sorted(str(path.relative_to(auth_root / "codex")) for path in (auth_root / "codex").rglob("*") if path.is_file()) == [".codex/auth.json", "auth.json"], "native logs or settings persisted"
        passed.append("actual pinned Codex synthetic API-key stdin login writer and local login-status reader; provider acceptance remains unverified")
        commit_test = Path(__file__).resolve().parent.parent / "tests/test_login_commit.py"
        commit = subprocess.run(["/usr/bin/python3", str(commit_test), "--binary", str(binary), "--run", "--required"],
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=90)
        print(commit.stdout.decode(errors="replace"), end="", flush=True)
        assert commit.returncode == 0, (commit.returncode, commit.stderr.decode(errors="replace"))
        passed.append("actual kernel ENOSPC rejects a partial host credential commit and retains private recovery credentials")
        insert = "INSERT INTO account(id,email,url,access_token,refresh_token,token_expiry,time_created,time_updated) VALUES ('fixture-account','fixture@invalid.example','https://example.invalid','fixture-access-v1','fixture-refresh-v1',1900000000000,0,0)"
        account_file = home / "data/agentcage/logins/opencode/.local/share/opencode/account.json"
        created = cli(["opencode", "--", "db", insert], timeout=120)
        assert created.returncode == 0, created.stderr.decode(errors="replace")
        stored = json.loads(account_file.read_text())
        assert stored["accounts"][0]["access_token"] == "fixture-access-v1", stored
        assert set(stored["accounts"][0]) == {"id", "email", "url", "access_token", "refresh_token", "token_expiry"}
        assert not (account_file.parent / "opencode.db").exists(), "full session database persisted"
        query = cli(["opencode", "--", "db", "SELECT access_token FROM account WHERE id='fixture-account'"], timeout=120)
        assert query.returncode == 0 and b"fixture-access-v1" in query.stdout + query.stderr, (query.returncode, query.stdout, query.stderr)
        changed = cli(["opencode", "--", "db", "UPDATE account SET access_token='fixture-access-v2',refresh_token='fixture-refresh-v2' WHERE id='fixture-account'"], timeout=120)
        assert changed.returncode == 0, changed.stderr.decode(errors="replace")
        refreshed = json.loads(account_file.read_text())["accounts"][0]
        assert refreshed["access_token"] == "fixture-access-v2" and refreshed["refresh_token"] == "fixture-refresh-v2"
        logout = cli(["opencode", "--", "console", "logout", "fixture@invalid.example"], timeout=120)
        assert logout.returncode == 0, logout.stderr.decode(errors="replace")
        assert not account_file.exists(), "synthetic Console account survived actual logout"
        assert b"\n  opencode\n" not in cli(["login", "list"]).stdout
        passed.append("actual OpenCode synthetic Console account INSERT/import/SELECT/token-update/export/console-logout, with full session DB remaining disposable")
        for signum in (signal.SIGINT, signal.SIGTERM):
            child = subprocess.Popen([str(binary), "shell"], cwd=project, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, start_new_session=True)
            try:
                child.stdin.write(b"python3 -c 'import signal,time; print(\"fixture SIGNAL READY\",flush=True); time.sleep(60)'\n")
                child.stdin.flush()
                read_until(child.stdout.fileno(), b"fixture SIGNAL READY")
                child.send_signal(signum)
                stdout, stderr = child.communicate(timeout=20)
                assert child.returncode == 128 + signum, (signum, child.returncode, stdout, stderr)
                assert not owned_ids(), owned_ids()
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.communicate(timeout=10)
            passed.append("actual process signal " + signal.Signals(signum).name + " forwards exit and tears down")
        master, slave = pty.openpty()
        child = subprocess.Popen([str(binary), "shell"], cwd=project, env=env | {"TERM": "xterm"}, stdin=slave, stdout=slave,
                                 stderr=slave, start_new_session=True)
        os.close(slave)
        try:
            os.write(master, b"python3 -c 'import time; print(\"fixture PTY READY\",flush=True); time.sleep(60)'\n")
            read_until(master, b"fixture PTY READY\r\n")
            os.write(master, b"\x03")
            os.write(master, b"exit 130\n")
            assert child.wait(timeout=20) == 130
            assert not owned_ids(), owned_ids()
            passed.append("actual PTY Ctrl-C followed by shell exit130 and teardown")
        finally:
            os.close(master)
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait(timeout=10)
        # These are all this test run's own disposable fixtures. The temporary
        # shim adds an extra ps label scope so even a default user store cannot
        # expose unrelated stopped containers to this destructive regression.
        def fixture(name, command, managed):
            options = ["create", "--name", name, "--label", label, "--read-only", "--cap-drop=ALL",
                       "--security-opt=no-new-privileges", "--userns=keep-id", "--entrypoint", "/bin/sh"]
            if managed:
                options.extend(["--label", "io.agentcage.managed=true"])
            made = raw([*options, "agentcage/agent-tools:local", "-c", command], timeout=120)
            assert made.returncode == 0, made.stderr
            identifier = made.stdout.decode().strip()
            started = raw(["start", identifier], timeout=120)
            assert started.returncode == 0, started.stderr
            if command == "exit 0":
                ended = raw(["wait", identifier], timeout=120)
                assert ended.returncode == 0 and ended.stdout.strip() == b"0", (ended.stdout, ended.stderr)
            return identifier

        def status(identifier):
            checked = raw(["inspect", "--format", "{{.State.Status}}", identifier])
            assert checked.returncode == 0, checked.stderr
            return checked.stdout.decode().strip()

        suffix = uuid.uuid4().hex[:8]
        stopped = fixture("agentcage-smoke-stopped-" + suffix, "exit 0", True)
        running = fixture("agentcage-smoke-running-" + suffix, "exec sleep 600", True)
        foreign_stopped = fixture("unrelated-agentcage-backup-" + suffix, "exit 0", False)
        foreign_running = fixture("agentcage-foreign-running-" + suffix, "exec sleep 600", False)
        cleaned = cli(["clean"], timeout=180, extra={"AC_SMOKE_CLEAN_SCOPE": "1"}, expect_teardown=False)
        assert cleaned.returncode == 0, cleaned.stderr
        assert stopped not in owned_ids(), "owned stopped fixture survived clean"
        assert status(running) == "running", "owned running fixture was stopped"
        assert status(foreign_running) == "running", "foreign running fixture was stopped"
        assert status(foreign_stopped) == "exited", "foreign stopped fixture was removed"
        passed.append("actual clean removes only managed stopped fixtures; managed running and foreign name matches survive (additional private smoke ps scope)")

        race = fixture("agentcage-smoke-restart-" + suffix, "exec sleep 600", True)
        stopped_race = raw(["stop", "--time", "1", race], timeout=120)
        assert stopped_race.returncode == 0 and status(race) == "exited", stopped_race.stderr
        with socket.socket(socket.AF_UNIX) as barrier:
            barrier.settimeout(180)
            barrier_path = str(root / "clean-barrier")
            barrier.bind(barrier_path)
            barrier.listen(1)
            child = subprocess.Popen([str(binary), "clean"], cwd=project, env=env | {"AC_SMOKE_CLEAN_SCOPE": "1",
                                     "AC_SMOKE_RACE_SOCKET": barrier_path, "AC_SMOKE_RACE_TARGET": race},
                                     stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
            try:
                channel, _ = barrier.accept()
                with channel:
                    with channel.makefile("rb") as stream:
                        assert json.loads(stream.readline())["target"] == race
                    restarted = raw(["start", race], timeout=120)
                    assert restarted.returncode == 0 and status(race) == "running", restarted.stderr
                    channel.sendall(b"release removal\n")
                stdout, stderr = child.communicate(timeout=120)
                assert child.returncode != 0, (stdout, stderr)
                assert status(race) == "running", "clean forcibly stopped a fixture restarted after selection"
            finally:
                if child.poll() is None:
                    os.killpg(child.pid, signal.SIGKILL)
                    child.communicate(timeout=10)
        passed.append("actual exited-to-running cleanup race, socket barrier before rm, nonzero cleanup and restarted process preserved")
        removed = raw(["rm", "-f", *owned_ids()], timeout=120)
        assert removed.returncode == 0 and not owned_ids(), removed.stderr
        print(json.dumps({"tested_binary_sha256": binary_sha, "real_smoke_passed": passed,
                          "upstream_oauth_refresh": "UNVERIFIED: no provider accounts or tokens supplied"}, indent=2), flush=True)
    finally:
        # Restore permissions only on this run's own intentional IO-fault
        # fixture so its TemporaryDirectory can remove leftover staging.
        for stage in temporary.glob("agentcage-login-codex-*"):
            nested = stage / ".codex"
            if nested.is_dir() and not nested.is_symlink():
                nested.chmod(0o700)
        # Only this run's unique disposable fixtures are eligible for removal.
        ids = owned_ids()
        if ids:
            removed = raw(["rm", "-f", *ids])
            assert removed.returncode == 0, removed.stderr
        assert not owned_ids(), "smoke teardown failed"

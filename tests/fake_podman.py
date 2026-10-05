#!/usr/bin/env python3
"""Disposable process/argv fixture; it does not claim container isolation."""
import fcntl
import json
import os
from pathlib import Path
import re
import signal
import socket
import sys
import uuid


root = Path(os.environ["FAKE_PODMAN_ROOT"])
args = sys.argv[1:]
(root / "calls" / str(uuid.uuid4())).write_text(json.dumps(args))


def update(fn):
    with (root / "lock").open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        state = json.loads((root / "state").read_text())
        result = fn(state)
        (root / "state").write_text(json.dumps(state))
        return result


def state():
    return update(lambda data: dict(data))


def output(text, code=0):
    print(text)
    sys.exit(code)


def failed(kind):
    data = state()
    if data.get(kind + "_failure"):
        print("fixture: " + kind + " failed", file=sys.stderr)
        sys.exit(125)


if args == ["--version"]:
    output("podman version 4.9.3")
if args[0] == "version":
    failed("version")
    output(state().get("version", "4.9.3"))
if args[0] == "info":
    failed("info")
    template = args[-1]
    data = state()
    if template.startswith('{"rootless":'):
        controllers = data.get("controllers", "[cpu memory pids]")
        if isinstance(controllers, str):
            controllers = controllers.strip("[]").split()
        snapshot = {"rootless": data.get("rootless", True), "remote": data.get("remote", False),
                    "runtime": data.get("runtime", "crun"),
                    "runtime_version": data.get("runtime_version", "crun version 1.14.1\ncommit: fixture"),
                    "cgroups": data.get("cgroups", "v2"), "manager": data.get("manager", "systemd"),
                    "controllers": controllers}
        output(json.dumps(data.get("host_snapshot", snapshot)))
    values = {
        "Rootless": str(data.get("rootless", True)).lower(),
        "OCIRuntime.Name": data.get("runtime", "crun"),
        "OCIRuntime.Version": data.get("runtime_version", "crun version 1.14.1\ncommit: fixture"),
        "ServiceIsRemote": str(data.get("remote", False)).lower(),
        "CgroupsVersion": data.get("cgroups", "v2"),
        "CgroupManager": data.get("manager", "systemd"),
        "CgroupControllers": data.get("controllers", "[cpu memory pids]"),
    }
    for key, value in values.items():
        if key in template:
            output(value)
    raise SystemExit("unrecognized info query: " + template)
if args[:2] == ["image", "exists"]:
    sys.exit(0 if state().get("image") else 1)
if args[:2] == ["image", "inspect"]:
    if ".Config.User" in args[-1]:
        output(str(os.getuid()) + ":" + str(os.getgid()))
    if "agentcage.dockerfile.hash" in args[-1]:
        output(state().get("image_hash", ""))
    raise SystemExit("unrecognized image query: " + args[-1])
if args[0] == "build":
    label = args[args.index("--label") + 1]
    update(lambda data: data.update(image=True, image_hash=label.split("=", 1)[1]))
    sys.exit(0)
if args[0] == "ps":
    failed("ps")
    filters = [args[i + 1] for i, arg in enumerate(args[:-1]) if arg == "--filter"]
    rows = []
    for item in state().get("containers", []):
        statuses = [f.split("=", 1)[1] for f in filters if f.startswith("status=")]
        if statuses and item["status"] not in statuses:
            continue
        if any(f.startswith("label=") and f[6:] not in item.get("labels", []) for f in filters):
            continue
        if any(f.startswith("name=") and not re.search(f[5:], item["name"]) for f in filters):
            continue
        rows.append(item["id"] if ".ID" in args[-1] else item["name"])
    if os.environ.get("FAKE_CLEAN_SOCKET"):
        with socket.socket(socket.AF_UNIX) as channel:
            channel.connect(os.environ["FAKE_CLEAN_SOCKET"])
            channel.sendall((json.dumps({"session": os.environ["FAKE_SESSION"]}) + "\n").encode())
            with channel.makefile("rb") as stream:
                stream.readline()
    output("\n".join(rows))
if args[0] == "rm":
    ids = [a for a in args[1:] if not a.startswith("-")]
    def remove(data):
        failures = []
        for identifier in ids:
            item = next((c for c in data.get("containers", []) if identifier in (c["id"], c["name"])), None)
            if not item:
                if "--ignore" not in args:
                    failures.append(identifier)
                continue
            if data.get("restart_on_remove") == item["id"]:
                item["status"] = "running"
            if item.get("rm_failure") or (item["status"] == "running" and "-f" not in args and "--force" not in args):
                failures.append(identifier)
            else:
                data["containers"].remove(item)
        return failures
    failures = update(remove)
    if failures:
        print("fixture: rm failed: " + ", ".join(failures), file=sys.stderr)
        sys.exit(125)
    sys.exit(0)
if args[0] == "run":
    stage = None
    for i, arg in enumerate(args[:-1]):
        if arg == "--mount":
            fields = dict(pair.split("=", 1) for pair in args[i + 1].split(",") if "=" in pair)
            if fields.get("dst", fields.get("destination")) == "/workspace/login-sync":
                stage = Path(fields["src"])
    if os.environ.get("FAKE_EXECUTE_WRAPPER"):
        # Metadata remains a fixture, but execute the binary's actual embedded
        # shell and credential helpers. Roots belong only to this test run.
        image_index = args.index("agentcage/agent-tools:local")
        command = args[image_index + 1:]
        if stage is not None and command[:2] != ["bash", "-c"]:
            raise SystemExit("fixture expected the production login wrapper")
        command = [str(stage) if value == "/workspace/login-sync" else value for value in command]
        # Like OCI JSON encoding, the actual runtime cannot preserve invalid
        # UTF-8 in its process strings. The live smoke verifies this boundary.
        command = [os.fsencode(value).decode("utf-8", "replace") for value in command]
        wrapper_env = dict(os.environ, HOME=os.environ["FAKE_CONTAINER_HOME"])
        os.execvpe(command[0], command, wrapper_env)
    control = os.environ.get("FAKE_CONTROL_SOCKET")
    if control:
        with socket.socket(socket.AF_UNIX) as channel:
            channel.connect(control)
            channel.sendall((json.dumps({"session": os.environ["FAKE_SESSION"], "staging": str(stage) if stage else None,
                                        "snapshot": (stage / "auth.json").read_text() if stage and (stage / "auth.json").exists() else None}) + "\n").encode())
            with channel.makefile("rb") as stream:
                action = json.loads(stream.readline())
            if "credential" in action:
                destination = stage / "auth.json"
                if action["credential"] is None:
                    destination.unlink(missing_ok=True)
                else:
                    destination.write_text(action["credential"])
                    destination.chmod(0o600)
            if action.get("symlink_parent"):
                destination = stage / ".codex"
                destination.mkdir(exist_ok=True)
                for child in destination.iterdir():
                    child.unlink()
                destination.rmdir()
                destination.symlink_to(action["symlink_parent"], target_is_directory=True)
            if action.get("fifo"):
                (stage / "auth.json").unlink(missing_ok=True)
                os.mkfifo(stage / "auth.json", 0o600)
            if action.get("oversize"):
                with (stage / "auth.json").open("wb") as dest:
                    dest.write(b"x" * (10 * 1024 * 1024 + 1))
    if stage is not None and not os.environ.get("FAKE_SKIP_COMPLETION"):
        # Direct-staging fixtures model a completed export. Tests of actual
        # copy-out use FAKE_EXECUTE_WRAPPER and the production helper instead.
        receipt = stage / ".agentcage-copy-complete"
        receipt.write_bytes(b"complete\n")
        receipt.chmod(0o600)
    if os.environ.get("FAKE_ECHO_STDIN"):
        if any(a in ("-i", "-it", "--interactive") for a in args):
            sys.stdout.buffer.write(sys.stdin.buffer.read())
    else:
        sys.stdout.write('{"fixture":"agent output"}\n')
    print("fixture: agent stderr", file=sys.stderr)
    sys.exit(int(os.environ.get("FAKE_EXIT", "0")))
raise SystemExit("unexpected fixture command: " + repr(args))

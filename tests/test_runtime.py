#!/usr/bin/env python3
"""Black-box CLI regressions, with synthetic credentials and explicit barriers."""
import argparse
import json
import os
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import tempfile
import time
import unittest
import uuid

from test_login_wrapper import (CODEX_OAUTH, INVALID_CODEX, INVALID_OPENCODE_ACCOUNTS,
                               VALID_OPENCODE_ACCOUNTS)


ROOT = Path(__file__).resolve().parents[1]
AC = None
OLD = '{"OPENAI_API_KEY":"fixture-old"}'
NEW_A = '{"OPENAI_API_KEY":"fixture-new-a"}'
NEW_B = '{"OPENAI_API_KEY":"fixture-new-b"}'


class RuntimeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-rt-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.project = self.base / "project with spaces"
        self.project.mkdir()
        self.home = self.base / "home"
        self.home.mkdir()
        self.tmp = self.base / "tmp"
        self.tmp.mkdir()
        fake = self.base / "bin"
        fake.mkdir()
        shutil.copyfile(ROOT / "tests/fake_podman.py", fake / "podman")
        (fake / "podman").chmod(0o755)
        self.fixture = self.base / "podman"
        self.fixture.mkdir()
        (self.fixture / "calls").mkdir()
        (self.fixture / "state").write_text("{}")
        self.env = {"PATH": str(fake) + ":/usr/bin:/bin", "HOME": str(self.home), "LANG": "C.UTF-8",
                    "XDG_DATA_HOME": str(self.home / "data"), "XDG_CONFIG_HOME": str(self.home / "config"),
                    "XDG_STATE_HOME": str(self.home / "state"), "XDG_CACHE_HOME": str(self.home / "cache"),
                    "TMPDIR": str(self.tmp), "FAKE_PODMAN_ROOT": str(self.fixture)}
        result = self.cli([])
        self.assertEqual(result.returncode, 0, result.stderr)

    def cli(self, args, extra=None, input=None):
        return subprocess.run([str(AC), *args], cwd=self.project, env=self.env | (extra or {}),
                              input=input, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)

    def calls(self, command=None):
        calls = [json.loads(p.read_text()) for p in (self.fixture / "calls").iterdir()]
        return [call for call in calls if command is None or call[0] == command]

    def state(self, **changes):
        path = self.fixture / "state"
        data = json.loads(path.read_text())
        if changes:
            data.update(changes)
            path.write_text(json.dumps(data))
        return data

    def credential(self, value=OLD):
        path = self.home / "data/agentcage/logins/codex/auth.json"
        path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
        path.write_text(value)
        path.chmod(0o600)
        return path

    def barrier(self):
        server = socket.socket(socket.AF_UNIX)
        server.settimeout(15)
        path = str(self.base / ("control-" + uuid.uuid4().hex[:6]))
        server.bind(path)
        server.listen(4)
        self.addCleanup(server.close)
        return server, path

    def begin(self, server, control, name, agent="codex", exit_code=0, extra=None):
        child = subprocess.Popen([str(AC), agent], cwd=self.project,
                                 env=self.env | (extra or {}) | {"FAKE_CONTROL_SOCKET": control, "FAKE_SESSION": name, "FAKE_EXIT": str(exit_code)},
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        def reap():
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
            child.communicate(timeout=5)
        self.addCleanup(reap)
        channel, _ = server.accept()
        channel.settimeout(15)
        self.addCleanup(channel.close)
        with channel.makefile("rb") as stream:
            ready = json.loads(stream.readline())
        self.assertEqual(ready["session"], name)
        return child, channel, ready

    def finish(self, session, action=None):
        child, channel, _ = session
        channel.sendall((json.dumps(action or {}) + "\n").encode())
        stdout, stderr = child.communicate(timeout=15)
        return child.returncode, stdout, stderr

    def assertNoStaging(self):
        self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def assert_conflict_recovery(self, session, returned, stderr):
        stage = Path(session[2]["staging"])
        message = ("failed to save login credentials for codex: login persistence conflict for codex: "
                   "another session changed the saved login; the newer store was kept; "
                   "returned credentials retained for recovery at ").encode() + os.fsencode(stage)
        self.assertIn(message, stderr)
        self.assertNotIn(b"fixture-new", stderr)
        self.assertTrue(stage.is_dir(), stderr)
        self.assertEqual((stage / ".agentcage-owner-pid").read_bytes(), b"save-failed")
        self.assertEqual((stage / ".agentcage-copy-complete").read_bytes(), b"complete\n")
        credential = stage / "auth.json"
        self.assertEqual(credential.read_bytes() if credential.exists() else None,
                         returned.encode() if returned is not None else None)
        for path in (stage, *stage.rglob("*")):
            info = path.lstat()
            self.assertEqual(info.st_uid, os.geteuid(), path)
            self.assertFalse(stat.S_ISLNK(info.st_mode), path)
            if stat.S_ISDIR(info.st_mode):
                self.assertEqual(info.st_mode & 0o777, 0o700, path)
            else:
                self.assertTrue(stat.S_ISREG(info.st_mode), path)
                self.assertEqual(info.st_mode & 0o777, 0o600, path)
                self.assertEqual(info.st_nlink, 1, path)
        return stage

    def assert_recovery_survives_cleanup_without_restoring(self, stage, expected, server, control):
        abandoned = self.tmp / ("agentcage-login-codex-abandoned-" + uuid.uuid4().hex)
        abandoned.mkdir(mode=0o700)
        marker = abandoned / ".agentcage-owner-pid"
        marker.write_bytes(b"0")
        marker.chmod(0o600)
        aged = time.time() - 72 * 60 * 60
        os.utime(abandoned, (aged, aged))
        os.utime(stage, (aged, aged))
        returned_before = {str(path.relative_to(stage)): path.read_bytes()
                           for path in stage.rglob("*") if path.is_file()}
        fresh = self.begin(server, control, "fresh-after-conflict")
        self.assertFalse(abandoned.exists(), "prepare-time cleanup did not remove its dead-owner control")
        self.assertEqual(fresh[2]["snapshot"], expected,
                         "a retained snapshot was automatically restored into a new session")
        code, _, stderr = self.finish(fresh)
        self.assertEqual(code, 0, stderr)
        path = self.home / "data/agentcage/logins/codex/auth.json"
        self.assertEqual(path.read_text() if path.exists() else None, expected)
        self.assertEqual({str(path.relative_to(stage)): path.read_bytes()
                          for path in stage.rglob("*") if path.is_file()}, returned_before)
        self.assertEqual(set(self.tmp.glob("agentcage-login-*")), {stage})
        listed = self.cli(["login", "list"])
        self.assertEqual(listed.returncode, 0, listed.stderr)
        self.assertEqual(b"\n  codex\n" in listed.stdout, expected is not None)

    def test_exit_codes_and_finalize(self):
        for status in (0, 1, 42, 125, 126, 127, 130):
            with self.subTest(status=status):
                path = self.credential()
                result = self.cli(["codex"], {"FAKE_EXIT": str(status)})
                self.assertEqual(result.returncode, status, result.stderr)
                self.assertEqual(path.read_text(), OLD)
                self.assertNoStaging()
                shell = subprocess.run(["/bin/sh", "-c", '"$AC_TEST_BINARY" codex --no-login && printf continued'],
                                       cwd=self.project, env=self.env | {"AC_TEST_BINARY": str(AC), "FAKE_EXIT": str(status)},
                                       stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                self.assertEqual(b"continued" in shell.stdout, status == 0)

    def test_pipe_and_separate_streams(self):
        payload = b"fixture input\x00\nline two\n"
        for args in (["codex", "--no-login", "--", "exec", "-"], ["shell"]):
            with self.subTest(args=args):
                result = self.cli(args, {"FAKE_ECHO_STDIN": "1"}, input=payload)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, payload)
                self.assertIn(b"AgentCage", result.stderr)
                for call in self.calls("run"):
                    self.assertTrue("-i" in call or "--interactive" in call)
                    self.assertFalse(any(arg in ("-t", "-it", "--tty") for arg in call))
        result = self.cli(["codex", "--no-login"])
        self.assertEqual(result.stdout, b'{"fixture":"agent output"}\n')

    def test_help_completions_and_login_list_work_from_deleted_cwd(self):
        self.credential()
        shim = "import os,sys; os.chdir(sys.argv[1]); os.rmdir(sys.argv[1]); os.execv(sys.argv[2],sys.argv[2:])"
        for arguments in (["--help"], ["help"], ["completions", "bash"],
                          ["completions", "zsh"], ["completions", "fish"], ["login", "list"]):
            with self.subTest(arguments=arguments):
                orphan = self.base / ("deleted-cwd-" + uuid.uuid4().hex)
                orphan.mkdir()
                result = subprocess.run(["/usr/bin/python3", "-c", shim, str(orphan), str(AC), *arguments],
                                        cwd=self.project, env=self.env, stdout=subprocess.PIPE,
                                        stderr=subprocess.PIPE, timeout=15)
                self.assertFalse(orphan.exists())
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(b"codex", result.stdout)
                self.assertNotIn(b"panicked", result.stderr)
                self.assertNoStaging()

    def test_help_and_completions_exit_with_sigpipe_when_stdout_is_closed(self):
        for arguments in (["--help"], ["help"], ["completions", "bash"],
                          ["completions", "zsh"], ["completions", "fish"]):
            with self.subTest(arguments=arguments):
                read_fd, write_fd = os.pipe()
                # Closing the reader before spawn removes any writer/reader race.
                os.close(read_fd)
                try:
                    child = subprocess.Popen([str(AC), *arguments], cwd=self.project, env=self.env,
                                             stdout=write_fd, stderr=subprocess.PIPE, restore_signals=True)
                finally:
                    os.close(write_fd)
                try:
                    _, stderr = child.communicate(timeout=15)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.communicate(timeout=5)
                    raise
                self.assertEqual(child.returncode, -signal.SIGPIPE, stderr)
                self.assertNotIn(b"panicked", stderr)
                self.assertNoStaging()

    def containers(self):
        def item(i, name, status, owned=True, failure=False):
            return {"id": format(i, "064x"), "name": name, "status": status,
                    "labels": ["io.agentcage.managed=true"] if owned else [], "rm_failure": failure}
        return [item(1, "agentcage-dead", "exited"), item(2, "agentcage-live", "running"),
                item(3, "unrelated-agentcage-backup", "exited", False), item(4, "agentcage-foreign", "exited", False),
                item(5, "owned-created", "created"), item(6, "owned-stopped", "stopped")]

    def test_clean_ownership_and_stopped_only(self):
        self.state(containers=self.containers())
        result = self.cli(["clean"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual({c["id"] for c in self.state()["containers"]}, {format(i, "064x") for i in (2, 3, 4)})
        for call in self.calls("rm"):
            self.assertNotIn("-f", call)
            self.assertNotIn("--force", call)
        result = self.cli(["clean"])
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_clean_reports_ps_and_partial_rm_failures(self):
        self.state(ps_failure=True)
        result = self.cli(["clean"])
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b"nothing to clean", result.stdout)
        self.assertIn(b"fixture: ps failed", result.stderr)
        containers = self.containers()
        containers[0]["rm_failure"] = True
        self.state(ps_failure=False, containers=containers)
        result = self.cli(["clean"])
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn(b"done\n", result.stdout)
        self.assertNotIn(format(5, "064x"), {c["id"] for c in self.state()["containers"]})
        self.assertIn(format(1, "064x"), {c["id"] for c in self.state()["containers"]})

    def test_clean_does_not_kill_container_restarted_after_listing(self):
        containers = self.containers()[:1]
        self.state(containers=containers, restart_on_remove=containers[0]["id"])
        result = self.cli(["clean"])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.state()["containers"][0]["status"], "running")

    def test_simultaneous_cleanups_are_idempotent(self):
        self.state(containers=self.containers())
        server, control = self.barrier()
        sessions = []
        for name in ("clean-a", "clean-b"):
            child = subprocess.Popen([str(AC), "clean"], cwd=self.project,
                                     env=self.env | {"FAKE_CLEAN_SOCKET": control, "FAKE_SESSION": name},
                                     stdout=subprocess.PIPE, stderr=subprocess.PIPE)
            self.addCleanup(lambda process=child: process.poll() is None and process.kill())
            channel, _ = server.accept()
            self.addCleanup(channel.close)
            with channel.makefile("rb") as stream:
                self.assertEqual(json.loads(stream.readline())["session"], name)
            sessions.append((child, channel))
        for child, channel in sessions:
            channel.sendall(b"release\n")
        for child, _ in sessions:
            _, stderr = child.communicate(timeout=15)
            self.assertEqual(child.returncode, 0, stderr)
        self.assertEqual({c["id"] for c in self.state()["containers"]}, {format(i, "064x") for i in (2, 3, 4)})

    def test_concurrent_refresh_and_unchanged_snapshot_both_orders(self):
        path = self.credential()
        for changed_first in (True, False):
            with self.subTest(changed_first=changed_first):
                path.write_text(OLD)
                server, control = self.barrier()
                a = self.begin(server, control, "A")
                b = self.begin(server, control, "B")
                self.assertEqual(a[2]["snapshot"], b[2]["snapshot"])
                for session, action in ((a, {"credential": NEW_A}), (b, {})) if changed_first else ((b, {}), (a, {"credential": NEW_A})):
                    status, _, stderr = self.finish(session, action)
                    self.assertEqual(status, 0, stderr)
                self.assertEqual(path.read_text(), NEW_A)
                self.assertNoStaging()
                server.close()
                Path(control).unlink()

    def test_two_distinct_refreshes_report_conflict_both_orders(self):
        for a_first in (True, False):
            for child_exit in (0, 42):
                with self.subTest(a_first=a_first, child_exit=child_exit):
                    path = self.credential()
                    server, control = self.barrier()
                    a = self.begin(server, control, "A", exit_code=0 if a_first else child_exit)
                    b = self.begin(server, control, "B", exit_code=child_exit if a_first else 0)
                    first, second = (a, b) if a_first else (b, a)
                    first_value, second_value = (NEW_A, NEW_B) if a_first else (NEW_B, NEW_A)
                    self.assertEqual(self.finish(first, {"credential": first_value})[0], 0)
                    status, _, stderr = self.finish(second, {"credential": second_value})
                    self.assertEqual(status, child_exit or 1, stderr)
                    self.assertEqual(path.read_text(), first_value)
                    stage = self.assert_conflict_recovery(second, second_value, stderr)
                    self.assert_recovery_survives_cleanup_without_restoring(stage, first_value, server, control)
                    shutil.rmtree(stage)
                    self.assertNoStaging()
                    server.close()
                    Path(control).unlink()

    def test_logout_and_refresh_conflict_both_orders(self):
        for logout_first in (True, False):
            for child_exit in (0, 42):
                with self.subTest(logout_first=logout_first, child_exit=child_exit):
                    path = self.credential()
                    server, control = self.barrier()
                    logout = self.begin(server, control, "logout", exit_code=0 if logout_first else child_exit)
                    refresh = self.begin(server, control, "refresh", exit_code=child_exit if logout_first else 0)
                    first, first_action, second, second_action = ((logout, {"credential": None}, refresh, {"credential": NEW_A})
                        if logout_first else (refresh, {"credential": NEW_A}, logout, {"credential": None}))
                    self.assertEqual(self.finish(first, first_action)[0], 0)
                    status, _, stderr = self.finish(second, second_action)
                    self.assertEqual(status, child_exit or 1, stderr)
                    expected = None if logout_first else NEW_A
                    self.assertEqual(path.read_text() if path.exists() else None, expected)
                    stage = self.assert_conflict_recovery(second, second_action["credential"], stderr)
                    self.assert_recovery_survives_cleanup_without_restoring(stage, expected, server, control)
                    shutil.rmtree(stage)
                    self.assertNoStaging()
                    server.close()
                    Path(control).unlink()

    def test_login_remove_revokes_two_open_snapshots_and_new_login_can_save(self):
        path = self.credential()
        server, control = self.barrier()
        a = self.begin(server, control, "A")
        b = self.begin(server, control, "B")
        result = self.cli(["login", "rm", "codex"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(path.exists())
        for session, action in ((a, {}), (b, {"credential": NEW_B})):
            _, _, stderr = self.finish(session, action)
            self.assertTrue(any(word in stderr.lower() for word in (b"removed", b"revok", b"generation", b"conflict")), stderr)
            self.assertFalse(path.exists())
        self.assertNotIn(b"\n  codex\n", self.cli(["login", "list"]).stdout)
        fresh = self.begin(server, control, "fresh")
        self.assertEqual(self.finish(fresh, {"credential": NEW_A})[0], 0)
        self.assertEqual(path.read_text(), NEW_A)
        self.assertNoStaging()

    def test_other_agent_is_not_blocked_by_open_session(self):
        self.credential()
        server, control = self.barrier()
        codex = self.begin(server, control, "codex")
        claude = self.begin(server, control, "claude", agent="claude")
        self.assertEqual(self.finish(claude)[0], 0)
        self.assertEqual(self.finish(codex)[0], 0)

    def test_unconfirmed_export_keeps_saved_login_even_when_staging_changes(self):
        for changed in (NEW_A, None):
            for status in (0, 42):
                with self.subTest(changed=changed is not None, status=status):
                    path = self.credential()
                    server, control = self.barrier()
                    session = self.begin(server, control, "incomplete", exit_code=status,
                                         extra={"FAKE_SKIP_COMPLETION": "1"})
                    code, _, stderr = self.finish(session, {"credential": changed})
                    self.assertEqual(code, 1 if status == 0 else status, stderr)
                    self.assertEqual(path.read_text() if path.exists() else None, OLD)
                    self.assertIn(b"credential export did not complete", stderr)
                    self.assertNoStaging()

    def test_signal_forwarding_finalizes_login_and_reports_interrupt(self):
        path = self.credential()
        for signum in (signal.SIGINT, signal.SIGTERM):
            with self.subTest(signum=signum):
                path.write_text(OLD)
                server, control = self.barrier()
                child, _, ready = self.begin(server, control, "signal")
                (Path(ready["staging"]) / "auth.json").write_text(NEW_A)
                # This direct-staging fixture models an export already fully
                # completed before Podman receives the forwarded signal.
                receipt = Path(ready["staging"]) / ".agentcage-copy-complete"
                receipt.write_bytes(b"complete\n")
                receipt.chmod(0o600)
                child.send_signal(signum)
                _, stderr = child.communicate(timeout=15)
                self.assertEqual(child.returncode, 128 + signum, stderr)
                self.assertEqual(path.read_text(), NEW_A)
                self.assertNoStaging()
                server.close()
                Path(control).unlink()

    def test_all_declared_agents_parse_help_and_forward_binary(self):
        help_output = self.cli(["--help"]).stdout
        for name, executable in (("codex", "codex"), ("claude", "claude"), ("opencode", "opencode"), ("antigravity", "agy")):
            with self.subTest(name=name):
                self.assertIn(name.encode(), help_output)
                result = self.cli([name, "--no-login", "--", "--help"])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertTrue(any(call[call.index("agentcage/agent-tools:local") + 1:] == [executable, "--help"] for call in self.calls("run")))

    def test_preflight_rejects_unsupported_runtime_before_run(self):
        cases = ({"rootless": False}, {"runtime": "runc"}, {"cgroups": "v1"},
                 {"controllers": "[cpu pids]"}, {"version": "3.0.0"}, {"manager": "cgroupfs"},
                 {"remote": True}, {"runtime_version": "crun version 1.8.0\ncommit: fixture"})
        for unsupported in cases:
            with self.subTest(unsupported=unsupported):
                self.state(**({"rootless": True, "runtime": "crun", "cgroups": "v2", "controllers": "[cpu memory pids]",
                               "version": "4.9.3", "manager": "systemd", "remote": False,
                               "runtime_version": "crun version 1.14.1\ncommit: fixture"} | unsupported))
                before = len(self.calls("run"))
                result = self.cli(["codex", "--no-login"])
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(self.calls("run")), before)

    def test_preflight_uses_one_coherent_typed_host_snapshot(self):
        before = len(self.calls("info"))
        result = self.cli(["codex", "--no-login"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("info")) - before, 1)
        for snapshot in ({}, {"rootless": "true"}, ["unrecognized"]):
            self.state(host_snapshot=snapshot)
            before = len(self.calls("run"))
            result = self.cli(["codex", "--no-login"])
            self.assertNotEqual(result.returncode, 0)
            self.assertIn(b"podman info", result.stderr)
            self.assertEqual(len(self.calls("run")), before)

    def test_agent_positional_arguments_survive_later_separator(self):
        for arguments in (["codex", "--no-login", "exec", "--", "-"], ["codex", "exec", "--no-login", "--", "-"]):
            before = set((self.fixture / "calls").iterdir())
            result = self.cli(arguments)
            self.assertEqual(result.returncode, 0, result.stderr)
            new_calls = [json.loads(path.read_text()) for path in set((self.fixture / "calls").iterdir()) - before]
            runs = [call for call in new_calls if call[0] == "run"]
            self.assertEqual(len(runs), 1)
            call = runs[0]
            self.assertEqual(call[call.index("agentcage/agent-tools:local") + 1:], ["codex", "exec", "-"])

    def test_non_utf8_agent_argument_is_forwarded_byte_for_byte(self):
        tool = Path(self.env["PATH"].split(":")[0]) / "codex"
        tool.write_text("""#!/usr/bin/env python3
import json,os,sys
print(json.dumps({'argv':[os.fsencode(value).hex() for value in sys.argv[1:]],
                  'stdin':sys.stdin.buffer.read().hex()}))
sys.exit(42 if sys.argv[1]=='exit42' else 0)
""")
        tool.chmod(0o755)
        container = self.base / "container-home"
        container.mkdir(mode=0o700)
        credential = self.credential()
        fixture = {"FAKE_EXECUTE_WRAPPER": "1", "FAKE_CONTAINER_HOME": str(container)}
        for no_login in (True, False):
            for status in (0, 42):
                with self.subTest(no_login=no_login, status=status):
                    tail = [b"exit42" if status else b"exec", b"fixture-\xff-\xfe", b"", b"--",
                            "žluťoučký".encode(), b"x" * 70000 + b"\xff"]
                    arguments = [b"codex", *([b"--no-login"] if no_login else []), b"--", *tail]
                    result = self.cli(arguments, fixture, input=b"fixture binary stdin\xff\n")
                    self.assertEqual(result.returncode, status, result.stderr)
                    actual = json.loads(result.stdout)
                    self.assertEqual(actual["argv"], [value.hex() for value in tail])
                    self.assertEqual(actual["stdin"], b"fixture binary stdin\xff\n".hex())
                    self.assertEqual(credential.read_text(), OLD)
                    self.assertNoStaging()
        result = self.cli([b"codex", b"fixture-\xff"])
        self.assertEqual(result.returncode, 1)
        self.assertIn(b"UTF-8", result.stderr)
        self.assertNotIn(b"panicked", result.stderr)

    def test_invalid_unicode_escape_is_not_listed_or_persisted(self):
        invalid = '{"primaryApiKey":"fixture\\u+061"}'
        path = self.home / "data/agentcage/logins/claude/.claude.json"
        path.parent.mkdir(parents=True, mode=0o700)
        path.write_text(invalid)
        listed = self.cli(["login", "list"])
        self.assertNotEqual(listed.returncode, 0, listed.stderr)
        self.assertNotIn(b"\n  claude\n", listed.stdout)
        path.write_text('{"primaryApiKey":"fixture-original"}')
        server, control = self.barrier()
        session = self.begin(server, control, "invalid-unicode", agent="claude")
        (Path(session[2]["staging"]) / ".claude.json").write_text(invalid)
        status, _, stderr = self.finish(session)
        self.assertNotEqual(status, 0, stderr)
        self.assertEqual(path.read_text(), '{"primaryApiKey":"fixture-original"}')
        self.assertNoStaging()

    def test_state_removal_refuses_symlink_parents_and_preserves_outside(self):
        outside = self.base / "outside-state"
        for name in ("fixture-env", "codex"):
            (outside / name).mkdir(parents=True)
            (outside / name / "sentinel").write_bytes(b"fixture outside state")
        envs = self.project / ".agentcage/envs"
        envs.symlink_to(outside, target_is_directory=True)
        for arguments in (["env", "rm", "fixture-env"], ["env", "list"]):
            result = self.cli(arguments)
            self.assertNotEqual(result.returncode, 0, result.stderr)
            self.assertEqual((outside / "fixture-env/sentinel").read_bytes(), b"fixture outside state")
        data = self.home / "data/agentcage"
        data.mkdir(parents=True)
        (data / "logins").symlink_to(outside, target_is_directory=True)
        result = self.cli(["login", "rm", "codex"])
        self.assertNotEqual(result.returncode, 0, result.stderr)
        self.assertEqual((outside / "codex/sentinel").read_bytes(), b"fixture outside state")

    def test_state_removal_unlinks_nested_symlinks_without_following_them(self):
        outside = self.base / "outside-nested"
        outside.mkdir()
        sentinel = outside / "sentinel"
        sentinel.write_bytes(b"fixture preserved")
        environment = self.project / ".agentcage/envs/fixture-env"
        (environment / "home/nested").mkdir(parents=True)
        (environment / "home/nested/link").symlink_to(outside, target_is_directory=True)
        result = self.cli(["env", "rm", "fixture-env"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(environment.exists())
        credential = self.credential()
        (credential.parent / "nested-link").symlink_to(outside, target_is_directory=True)
        result = self.cli(["login", "rm", "codex"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(credential.parent.exists())
        self.assertEqual(sentinel.read_bytes(), b"fixture preserved")

    def test_revocation_after_a_completed_refresh_blocks_older_session(self):
        path = self.credential()
        server, control = self.barrier()
        changed = self.begin(server, control, "completed-refresh")
        older = self.begin(server, control, "older-session")
        self.assertEqual(self.finish(changed, {"credential": NEW_A})[0], 0)
        self.assertEqual(path.read_text(), NEW_A)
        self.assertEqual(self.cli(["login", "rm", "codex"]).returncode, 0)
        result, _, stderr = self.finish(older)
        self.assertNotEqual(result, 0, stderr)
        self.assertFalse(path.exists())
        self.assertNoStaging()

    def test_preflight_preserves_failure_diagnostic(self):
        for kind in ("version", "info"):
            with self.subTest(kind=kind):
                self.state(**({"version_failure": False, "info_failure": False} | {kind + "_failure": True}))
                result = self.cli(["codex", "--no-login"])
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(("fixture: " + kind + " failed").encode(), result.stderr)

    def test_login_json_whitespace_types_and_nested_keys(self):
        path = self.home / "data/agentcage/logins/opencode/.local/share/opencode/account.json"
        path.parent.mkdir(parents=True, mode=0o700)
        for description, document in VALID_OPENCODE_ACCOUNTS.items():
            with self.subTest(content=description):
                content = ("\n " + json.dumps(document, ensure_ascii=True, indent=3) + "\n").encode()
                path.write_bytes(content)
                path.chmod(0o600)
                result = self.cli(["login", "list"])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(b"\n  opencode\n" in result.stdout, bool(document["accounts"]))
                self.assertEqual(path.read_bytes(), content)
                self.assertNoStaging()

    def test_invalid_opencode_account_is_not_listed_or_started(self):
        path = self.home / "data/agentcage/logins/opencode/.local/share/opencode/account.json"
        path.parent.mkdir(parents=True, mode=0o700)
        for description, content in INVALID_OPENCODE_ACCOUNTS.items():
            with self.subTest(content=description):
                path.write_bytes(content)
                path.chmod(0o600)
                listed = self.cli(["login", "list"])
                self.assertEqual(listed.returncode, 1, listed.stderr)
                self.assertNotIn(b"\n  opencode\n", listed.stdout)
                self.assertEqual(path.read_bytes(), content)
                runs = self.calls("run")
                result = self.cli(["opencode"])
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertEqual(self.calls("run"), runs, "invalid account snapshot reached the agent")
                self.assertEqual(path.read_bytes(), content)
                self.assertNotIn(b"fixture-access", result.stderr)
                self.assertNotIn(b"fixture-refresh", result.stderr)
                self.assertNoStaging()

    def test_login_list_rejects_invalid_codex_auth_dot_json(self):
        path = self.credential()
        for description, invalid in INVALID_CODEX.items():
            with self.subTest(content=description):
                path.write_bytes(invalid)
                result = self.cli(["login", "list"])
                self.assertEqual((result.returncode != 0, b"\n  codex\n" in result.stdout, path.read_bytes()),
                                 (True, False, invalid), result.stderr)
                self.assertNoStaging()

    def test_login_list_accepts_synthetic_codex_api_and_complete_oauth_documents(self):
        path = self.credential()
        for valid in (OLD.encode(), b"\n " + json.dumps(CODEX_OAUTH).encode() + b"\n"):
            with self.subTest(content=valid):
                path.write_bytes(valid)
                result = self.cli(["login", "list"])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn(b"\n  codex\n", result.stdout)
                self.assertEqual(path.read_bytes(), valid)
                self.assertNoStaging()

    def test_private_permissions_and_unsafe_staging_are_rejected(self):
        path = self.credential()
        outside = self.base / "outside"
        outside.mkdir()
        sentinel = outside / "auth.json"
        sentinel.write_text("fixture sentinel")
        for action in ({"symlink_parent": str(outside)}, {"fifo": True}, {"oversize": True}):
            with self.subTest(action=action):
                path.write_text(OLD)
                server, control = self.barrier()
                session = self.begin(server, control, "unsafe")
                status, _, _ = self.finish(session, action)
                self.assertNotEqual(status, 0)
                self.assertEqual(sentinel.read_text(), "fixture sentinel")
                self.assertEqual(path.read_text(), OLD)
                self.assertEqual(path.stat().st_mode & 0o777, 0o600)
                self.assertEqual(path.parent.stat().st_mode & 0o777, 0o700)
                self.assertNoStaging()
                server.close()
                Path(control).unlink()

    def test_child_failure_is_preserved_when_login_finalize_also_fails(self):
        path = self.credential()
        server, control = self.barrier()
        session = self.begin(server, control, "dual-failure", exit_code=42)
        status, _, stderr = self.finish(session, {"oversize": True})
        self.assertEqual(status, 42, stderr)
        self.assertTrue(any(word in stderr.lower() for word in (b"large", b"size", b"bound")), stderr)
        self.assertEqual(path.read_text(), OLD)
        self.assertNoStaging()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args, remaining = parser.parse_known_args()
    input_binary = args.binary.resolve()
    if os.name != "posix" or not input_binary.is_file():
        parser.error("a built Linux ac binary is required")
    with tempfile.TemporaryDirectory(prefix="ac-test-binary-") as binary_dir:
        AC = Path(binary_dir) / "ac"
        shutil.copy2(input_binary, AC)
        unittest.main(argv=[__file__, *remaining])

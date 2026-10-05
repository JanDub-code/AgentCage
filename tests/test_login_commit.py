#!/usr/bin/env python3
"""Opt-in Linux kernel ENOSPC regression for whole-store login publication."""
import argparse
from contextlib import contextmanager
import errno
import hashlib
import json
import os
from pathlib import Path
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
import uuid

import test_runtime as runtime


OWNER = ".agentcage-owner-pid"
NAMESPACE_VERIFIED = False
FIRST = b'{"OPENAI_API_KEY":"fixture-returned-first"}'
SECOND = json.dumps({"OPENAI_API_KEY": "fixture-returned-second-" + "x" * (512 * 1024)},
                    separators=(",", ":")).encode()
FRESH = (b'{"OPENAI_API_KEY":"fixture-fresh-first"}',
         b'{"OPENAI_API_KEY":"fixture-fresh-second"}')


class Unavailable(Exception):
    pass


class CommitTests(unittest.TestCase):
    def setUp(self):
        if not NAMESPACE_VERIFIED:
            self.skipTest("kernel scenario requires the opt-in namespace harness: --run")

    @contextmanager
    def filesystem(self):
        fixture = runtime.RuntimeTests("runTest")
        fixture.setUp()
        try:
            mountpoint = fixture.home / "data/agentcage/logins"
            mountpoint.mkdir(parents=True, mode=0o700)
            mounted = subprocess.run(["mount", "-t", "tmpfs", "-o", "size=1m,mode=0700,nosuid,nodev",
                                      "tmpfs", str(mountpoint)], capture_output=True, timeout=10)
            if mounted.returncode:
                raise Unavailable("private tmpfs mount unavailable: " + mounted.stderr.decode(errors="replace"))

            def unmount():
                # This exact mount exists only in this user/mount namespace.
                result = subprocess.run(["umount", "--", str(mountpoint)], capture_output=True, timeout=10)
                self.assertEqual(result.returncode, 0, result.stderr)

            # Runtime helpers register process/socket cleanup later, so they run
            # before this unmount and the original TemporaryDirectory cleanup.
            fixture.addCleanup(unmount)
            mounted_entry = [line for line in Path("/proc/self/mountinfo").read_text().splitlines()
                             if line.split()[4] == str(mountpoint)]
            self.assertEqual(len(mounted_entry), 1)
            self.assertEqual(mounted_entry[0].split(" - ")[1].split()[0], "tmpfs")
            capacity = os.statvfs(mountpoint)
            self.assertEqual(capacity.f_blocks * capacity.f_frsize, 1024 * 1024)
            old = runtime.OLD.encode()
            store = mountpoint / "codex"
            (store / ".codex").mkdir(parents=True, mode=0o700)
            for relative in ("auth.json", ".codex/auth.json"):
                path = store / relative
                path.write_bytes(old)
                path.chmod(0o600)
            yield fixture, mountpoint, store, old
        finally:
            self.assertTrue(fixture.doCleanups(), "owned filesystem/process cleanup failed")

    def start_returned(self, fixture, status, values=(FIRST, SECOND)):
        server, control = fixture.barrier()
        session = fixture.begin(server, control, "commit-" + uuid.uuid4().hex[:8], exit_code=status)
        staging = Path(session[2]["staging"])
        self.assertEqual(staging.parent, fixture.tmp)
        self.assertTrue(staging.name.startswith("agentcage-login-codex-"))
        for relative, value in zip(("auth.json", ".codex/auth.json"), values):
            destination = staging / relative
            destination.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
            destination.write_bytes(value)
            destination.chmod(0o600)
        return session, staging

    def assert_bytes(self, path, expected):
        actual = path.read_bytes()
        # Keep failures bounded even for the deliberately large valid document.
        self.assertEqual((len(actual), hashlib.sha256(actual).hexdigest()),
                         (len(expected), hashlib.sha256(expected).hexdigest()), str(path))

    def assert_private_recovery(self, staging, values, stderr=None):
        if stderr is not None:
            self.assertIn(b"returned credentials retained for recovery at " + os.fsencode(staging), stderr)
        self.assertTrue(staging.is_dir())
        for directory in (staging, staging / ".codex"):
            metadata = directory.stat()
            self.assertEqual((stat.S_IMODE(metadata.st_mode), metadata.st_uid), (0o700, os.getuid()))
        for relative, value in zip(("auth.json", ".codex/auth.json"), values):
            path = staging / relative
            self.assert_bytes(path, value)
            metadata = path.stat()
            self.assertTrue(stat.S_ISREG(metadata.st_mode))
            self.assertEqual((stat.S_IMODE(metadata.st_mode), metadata.st_uid, metadata.st_nlink),
                             (0o600, os.getuid(), 1))
        self.assert_bytes(staging / OWNER, b"save-failed")
        self.assertEqual(stat.S_IMODE((staging / OWNER).stat().st_mode), 0o600)

    def fill_and_prove_kernel_enospc(self, mountpoint):
        available = os.statvfs(mountpoint)
        free_bytes = available.f_bavail * available.f_frsize
        filler = mountpoint / "owned-space-filler"
        filler.write_bytes(b"x" * (free_bytes - 128 * 1024))
        filler.chmod(0o600)
        probe = mountpoint / "owned-enospc-probe"
        descriptor = os.open(probe, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        observed = None
        try:
            for _ in range(32):
                try:
                    os.write(descriptor, b"p" * (64 * 1024))
                except OSError as error:
                    observed = error.errno
                    break
            self.assertEqual(observed, errno.ENOSPC, "kernel did not enforce the private 1MiB tmpfs")
        finally:
            os.close(descriptor)
            probe.unlink()
        remaining = os.statvfs(mountpoint)
        self.assertLess(remaining.f_bavail * remaining.f_frsize, len(SECOND))
        print("KERNEL_ENOSPC", json.dumps({"errno": observed, "capacity_bytes": 1024 * 1024,
                                         "remaining_bytes": remaining.f_bavail * remaining.f_frsize}), flush=True)
        return filler

    def test_kernel_enospc_preserves_store_and_recoverable_snapshot(self):
        for child_status in (0, 42):
            with self.subTest(child_status=child_status), self.filesystem() as (fixture, mountpoint, store, old):
                session, staging = self.start_returned(fixture, child_status)
                filler = self.fill_and_prove_kernel_enospc(mountpoint)
                status, _, stderr = fixture.finish(session)
                self.assertEqual(status, 1 if child_status == 0 else 42, stderr)
                self.assertIn(b"No space left on device", stderr)
                for relative in ("auth.json", ".codex/auth.json"):
                    self.assert_bytes(store / relative, old)
                self.assert_private_recovery(staging, (FIRST, SECOND), stderr)
                self.assertEqual(list(mountpoint.glob(".agentcage-save-*")), [])

                # An aged recovery copy must survive actual prepare-time cleanup.
                # A control with a dead owner proves that cleanup really ran.
                abandoned = fixture.tmp / ("agentcage-login-codex-abandoned-" + uuid.uuid4().hex)
                abandoned.mkdir(mode=0o700)
                (abandoned / OWNER).write_bytes(b"0")
                (abandoned / OWNER).chmod(0o600)
                aged = time.time() - 72 * 60 * 60
                os.utime(abandoned, (aged, aged))
                os.utime(staging, (aged, aged))
                unchanged = fixture.cli(["codex"])
                self.assertEqual(unchanged.returncode, 0, unchanged.stderr)
                self.assertFalse(abandoned.exists(), "stale cleanup control was not removed")
                self.assert_private_recovery(staging, (FIRST, SECOND))

                filler.unlink()
                self.assert_private_recovery(staging, (FIRST, SECOND))
                fresh, _ = self.start_returned(fixture, 0, FRESH)
                fresh_status, _, fresh_stderr = fixture.finish(fresh)
                self.assertEqual(fresh_status, 0, fresh_stderr)
                for relative, value in zip(("auth.json", ".codex/auth.json"), FRESH):
                    self.assert_bytes(store / relative, value)
                    self.assertEqual(stat.S_IMODE((store / relative).stat().st_mode), 0o600)
                self.assert_private_recovery(staging, (FIRST, SECOND))
                self.assertEqual(set(fixture.tmp.glob("agentcage-login-*")), {staging})
                print("PASS: ENOSPC child", child_status, "whole OLD store, private retained recovery,",
                      "aged cleanup protection and fresh save after freeing space", flush=True)

    def test_unmanaged_store_file_is_preserved(self):
        for child_status in (0, 42):
            with self.subTest(child_status=child_status), self.filesystem() as (fixture, mountpoint, store, old):
                sentinel = store / "fixture-unmanaged-file"
                sentinel.write_bytes(b"fixture unmanaged sentinel")
                sentinel.chmod(0o600)
                session, staging = self.start_returned(fixture, child_status, FRESH)
                status, _, stderr = fixture.finish(session)
                self.assertEqual(status, 1 if child_status == 0 else 42, stderr)
                self.assertIn(b"unrecognized saved login entry", stderr)
                self.assert_bytes(sentinel, b"fixture unmanaged sentinel")
                for relative in ("auth.json", ".codex/auth.json"):
                    self.assert_bytes(store / relative, old)
                self.assert_private_recovery(staging, FRESH, stderr)
                self.assertEqual(list(mountpoint.glob(".agentcage-save-*")), [])
                print("PASS: unmanaged file and entire store preserved for child", child_status, flush=True)


def main():
    global NAMESPACE_VERIFIED
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--run", action="store_true", help="run owned user/mount namespace and private 1MiB tmpfs")
    parser.add_argument("--required", action="store_true", help="unavailable runtime fails instead of exit 77")
    parser.add_argument("--_child", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--_parent-user-ns", help=argparse.SUPPRESS)
    parser.add_argument("--_parent-mount-ns", help=argparse.SUPPRESS)
    args = parser.parse_args()

    def unavailable(reason):
        print("UNVERIFIED: " + reason, flush=True)
        return 1 if args.required else 77

    if not args.run:
        return unavailable("kernel namespace test is opt-in; invoke --run (release gate also needs --required)")
    if not sys.platform.startswith("linux") or not args.binary.is_file():
        return unavailable("Linux and a built ac binary are required")
    if args._child:
        if (not args._parent_user_ns or not args._parent_mount_ns or
                os.readlink("/proc/self/ns/user") == args._parent_user_ns or
                os.readlink("/proc/self/ns/mnt") == args._parent_mount_ns):
            return unavailable("a new user and mount namespace is required")
        NAMESPACE_VERIFIED = True
        fixtures = args._child / "fixtures"
        fixtures.mkdir(mode=0o700)
        tempfile.tempdir = str(fixtures)
        runtime.AC = args.binary.resolve()
        print("NAMESPACE_READY", json.dumps({"uid": os.getuid(), "user": os.readlink("/proc/self/ns/user"),
                                             "mount": os.readlink("/proc/self/ns/mnt"),
                                             "kernel": os.uname().release,
                                             "oci": "synthetic fixture; real ac finalization and kernel IO"}), flush=True)
        suite = unittest.defaultTestLoader.loadTestsFromTestCase(CommitTests)
        result = unittest.TextTestRunner(verbosity=2).run(suite)
        if any("Unavailable:" in error for _, error in result.errors):
            return unavailable("private kernel tmpfs mount unavailable; see diagnostics above")
        return 0 if result.wasSuccessful() else 1

    if not all(shutil.which(command) for command in ("unshare", "mount", "umount")):
        return unavailable("unshare, mount and umount are required")
    # Only newly created /tmp fixtures are written or mounted. The user's HOME
    # and runtime configuration are never used by the injected CLI fixtures.
    with tempfile.TemporaryDirectory(prefix="ac-login-commit-", dir="/tmp") as directory:
        base = Path(directory)
        binary = base / "ac"
        shutil.copy2(args.binary.resolve(), binary)
        binary.chmod(0o500)
        command = ["unshare", "--user", "--map-root-user", "--mount", "--pid", "--fork", "--kill-child",
                   "--propagation", "private",
                   "--", sys.executable, str(Path(__file__).resolve()), "--binary", str(binary), "--run",
                   "--_child", str(base), "--_parent-user-ns", os.readlink("/proc/self/ns/user"),
                   "--_parent-mount-ns", os.readlink("/proc/self/ns/mnt")]
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, start_new_session=True)
        try:
            output, _ = child.communicate(timeout=55)
        except subprocess.TimeoutExpired:
            # Killing the owned namespace's PID 1 also kills its CLI descendants.
            os.killpg(child.pid, signal.SIGKILL)
            output, _ = child.communicate(timeout=5)
            sys.stdout.buffer.write(output)
            print("FAIL: owned kernel namespace regression timed out", flush=True)
            return 1
        sys.stdout.buffer.write(output)
        sys.stdout.flush()
        if b"NAMESPACE_READY" not in output:
            return unavailable("user/mount namespace creation unavailable; see diagnostics above")
        return (1 if args.required else 77) if child.returncode == 77 else child.returncode


if __name__ == "__main__":
    raise SystemExit(main())

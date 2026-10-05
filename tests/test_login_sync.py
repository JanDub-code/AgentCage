#!/usr/bin/env python3
"""Real FD-based transfer regressions; every credential and sentinel is artificial."""
import os
import errno
import importlib.util
from pathlib import Path
import shutil
import stat
import subprocess
import tempfile
import unittest
from unittest import mock


SOURCE = (Path(__file__).resolve().parents[1] / "src/login_sync.py").read_text()
SHAPES = (("auth.json", ".codex/auth.json"), (".claude/.credentials.json", ".claude.json"),
          (".local/share/opencode/auth.json", ".local/share/opencode/account.json"),
          (".gemini/antigravity-cli/antigravity-oauth-token",))


class LoginSyncTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-sync-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.source = self.root / "source"
        self.destination = self.root / "destination"
        self.outside = self.root / "outside"
        for directory in (self.source, self.destination, self.outside):
            directory.mkdir(mode=0o700)
        self.sentinel = self.write(self.outside, "auth.json", b"outside artificial credential sentinel")
        self.sentinel.chmod(0o644)

    def write(self, root, relative, content):
        path = root / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(content)
        return path

    def invoke(self, paths, maximum=1024, reverse=False, export=False):
        source, destination = (self.destination, self.source) if reverse else (self.source, self.destination)
        mode = ["--export"] if export else []
        return subprocess.run(["python3", "-c", SOURCE, *mode, str(source), str(destination), str(maximum), *paths],
                              env={"PATH": "/usr/bin:/bin", "HOME": str(self.root), "TMPDIR": str(self.root), "LANG": "C.UTF-8"},
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)

    def protected(self):
        self.assertEqual(self.sentinel.read_bytes(), b"outside artificial credential sentinel")
        self.assertEqual(self.sentinel.stat().st_mode & 0o777, 0o644)
        self.assertEqual(list(self.destination.rglob(".agentcage-auth-*")), [])

    def test_all_agent_shapes_binary_roundtrip_and_private_created_parents(self):
        for shape in SHAPES:
            with self.subTest(shape=shape):
                for path in shape:
                    self.write(self.source, path, b"\xff\x00fixture binary\n")
                result = self.invoke(shape)
                self.assertEqual(result.returncode, 0, result.stderr)
                for path in shape:
                    destination = self.destination / path
                    self.assertEqual(destination.read_bytes(), b"\xff\x00fixture binary\n")
                    self.assertEqual(destination.stat().st_mode & 0o777, 0o600)
                    parent = destination.parent
                    while parent != self.destination:
                        self.assertEqual(parent.stat().st_mode & 0o777, 0o700)
                        parent = parent.parent
                    destination.write_bytes(b"updated synthetic token")
                self.assertEqual(self.invoke(shape, reverse=True).returncode, 0)
                for path in shape:
                    self.assertEqual((self.source / path).read_bytes(), b"updated synthetic token")
        self.protected()

    def test_genuine_absence_deletes_only_allowlisted_destination(self):
        self.write(self.destination, ".codex/auth.json", b"old synthetic credential")
        unrelated = self.write(self.destination, "unrelated", b"unrelated fixture")
        self.assertEqual(self.invoke([".codex/auth.json"]).returncode, 0)
        self.assertFalse((self.destination / ".codex/auth.json").exists())
        self.assertEqual(unrelated.read_bytes(), b"unrelated fixture")
        self.assertEqual(self.invoke([".codex/auth.json"]).returncode, 0)
        self.protected()

    def test_export_completion_follows_complete_update_and_logout(self):
        paths = ("auth.json", ".codex/auth.json")
        receipt = self.destination / ".agentcage-copy-complete"
        unrelated = self.write(self.destination, "unrelated", b"unrelated fixture")
        for relative in paths:
            self.write(self.source, relative, b"new synthetic credential")
            self.write(self.destination, relative, b"old synthetic credential")
        result = self.invoke(paths, export=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        for relative in paths:
            self.assertEqual((self.destination / relative).read_bytes(), b"new synthetic credential")
        self.assertEqual(receipt.read_bytes(), b"complete\n")
        info = receipt.stat()
        self.assertTrue(stat.S_ISREG(info.st_mode))
        self.assertEqual((info.st_mode & 0o777, info.st_nlink), (0o600, 1))

        # A second successful export replaces a previous completion only
        # after the update and genuine deletion both took effect.
        (self.source / "auth.json").unlink()
        (self.source / ".codex/auth.json").write_bytes(b"refreshed synthetic credential")
        result = self.invoke(paths, export=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.destination / "auth.json").exists())
        self.assertEqual((self.destination / ".codex/auth.json").read_bytes(), b"refreshed synthetic credential")
        self.assertEqual(receipt.read_bytes(), b"complete\n")
        (self.source / ".codex/auth.json").unlink()
        result = self.invoke(paths, export=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(all(not (self.destination / relative).exists() for relative in paths))
        self.assertEqual(receipt.read_bytes(), b"complete\n")
        self.assertEqual(unrelated.read_bytes(), b"unrelated fixture")
        self.protected()

    def test_export_kernel_eacces_cannot_leave_fresh_or_stale_completion(self):
        if os.geteuid() == 0:
            self.skipTest("real permission denial requires an unprivileged fixture UID")
        paths = ("auth.json", ".codex/auth.json")
        receipt = self.destination / ".agentcage-copy-complete"
        for previous_export in (False, True):
            with self.subTest(previous_export=previous_export):
                for relative in paths:
                    self.write(self.source, relative, b"old synthetic credential")
                    self.write(self.destination, relative, b"old synthetic credential")
                if previous_export:
                    completed = self.invoke(paths, export=True)
                    self.assertEqual(completed.returncode, 0, completed.stderr)
                    self.assertTrue(receipt.is_file())
                else:
                    self.assertFalse(receipt.exists())
                for relative in paths:
                    (self.source / relative).unlink()
                blocked = self.destination / ".codex"
                blocked.chmod(0o500)
                try:
                    # Check the real kernel restriction on this own fixture;
                    # subprocess transfer uses the same ordinary unlink.
                    with self.assertRaises(PermissionError) as denied:
                        (blocked / "auth.json").unlink()
                    self.assertEqual(denied.exception.errno, errno.EACCES)
                    result = self.invoke(paths, export=True)
                    self.assertEqual(result.returncode, 1, result.stderr)
                    self.assertFalse((self.destination / "auth.json").exists())
                    self.assertEqual((blocked / "auth.json").read_bytes(), b"old synthetic credential")
                    self.assertFalse(receipt.exists(), "failed partial export retained a completion receipt")
                    self.assertNotIn(b"old synthetic credential", result.stderr)
                    self.protected()
                finally:
                    blocked.chmod(0o700)

    def test_source_parent_symlink_and_dangling_parent_preserve_existing_login(self):
        destination = self.write(self.destination, ".codex/auth.json", b"old synthetic credential")
        for target in (self.outside, self.root / "missing"):
            with self.subTest(target=target):
                link = self.source / ".codex"
                link.symlink_to(target, target_is_directory=True)
                result = self.invoke([".codex/auth.json"])
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(destination.read_bytes(), b"old synthetic credential")
                self.assertNotIn(b"outside artificial", result.stderr)
                link.unlink()
                self.protected()

    def test_every_nested_source_and_destination_parent_rejects_redirection(self):
        relative = ".local/share/opencode/auth.json"
        for side in (self.source, self.destination):
            for parent in (".local", ".local/share", ".local/share/opencode"):
                with self.subTest(side=side.name, parent=parent):
                    self.write(self.source, relative, b"new synthetic credential")
                    existing = self.write(self.destination, relative, b"old synthetic credential")
                    redirect = side / parent
                    backup = self.root / "backup"
                    redirect.rename(backup)
                    redirect.symlink_to(self.outside, target_is_directory=True)
                    result = self.invoke([relative])
                    self.assertNotEqual(result.returncode, 0)
                    redirect.unlink()
                    backup.rename(redirect)
                    self.assertEqual(existing.read_bytes(), b"old synthetic credential")
                    self.protected()

    def test_symlink_hardlink_fifo_and_oversize_sources_do_not_change_destination(self):
        old = self.write(self.destination, "auth.json", b"old synthetic credential")
        for kind in ("symlink", "dangling", "hardlink", "fifo", "oversize"):
            with self.subTest(kind=kind):
                path = self.source / "auth.json"
                if kind in ("symlink", "dangling"):
                    path.symlink_to(self.sentinel if kind == "symlink" else self.root / "missing")
                elif kind == "hardlink":
                    os.link(self.sentinel, path)
                elif kind == "fifo":
                    os.mkfifo(path)
                else:
                    path.write_bytes(b"x" * 1025)
                result = self.invoke(["auth.json"])
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(old.read_bytes(), b"old synthetic credential")
                path.unlink()
                self.protected()

    def test_destination_leaf_symlink_hardlink_fifo_and_dangling_parent_are_rejected(self):
        self.write(self.source, ".codex/auth.json", b"new synthetic credential")
        for kind in ("symlink", "hardlink", "fifo", "dangling-parent"):
            with self.subTest(kind=kind):
                folder = self.destination / ".codex"
                if kind == "dangling-parent":
                    folder.symlink_to(self.root / "missing", target_is_directory=True)
                else:
                    folder.mkdir()
                    path = folder / "auth.json"
                    if kind == "symlink":
                        path.symlink_to(self.sentinel)
                    elif kind == "hardlink":
                        os.link(self.sentinel, path)
                    else:
                        os.mkfifo(path)
                result = self.invoke([".codex/auth.json"])
                self.assertNotEqual(result.returncode, 0)
                if kind == "dangling-parent":
                    folder.unlink()
                else:
                    (folder / "auth.json").unlink()
                    folder.rmdir()
                self.protected()

    def test_later_source_or_destination_failure_keeps_earlier_credential_unchanged(self):
        for side in (self.source, self.destination):
            with self.subTest(side=side.name):
                self.write(self.source, "auth.json", b"new first credential")
                first = self.write(self.destination, "auth.json", b"old first credential")
                self.write(self.source, ".codex/auth.json", b"new second credential")
                second = self.write(self.destination, ".codex/auth.json", b"old second credential")
                parent = side / ".codex"
                shutil.rmtree(parent)
                parent.symlink_to(self.root / "missing", target_is_directory=True)
                result = self.invoke(["auth.json", ".codex/auth.json"])
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(first.read_bytes(), b"old first credential")
                if side == self.source:
                    self.assertEqual(second.read_bytes(), b"old second credential")
                parent.unlink()
                parent.mkdir()
                self.protected()

    def test_root_symlinks_and_unsafe_relative_components_are_rejected(self):
        first = self.write(self.destination, "auth.json", b"old fixture")
        for relative in ("../auth.json", "/auth.json", "./auth.json", ".codex//auth.json", "auth.json/", "bad name/auth.json"):
            with self.subTest(relative=relative):
                self.assertNotEqual(self.invoke([relative]).returncode, 0)
                self.assertEqual(first.read_bytes(), b"old fixture")
        real = self.root / "real-source"
        self.source.rename(real)
        self.source.symlink_to(real, target_is_directory=True)
        self.assertNotEqual(self.invoke(["auth.json"]).returncode, 0)
        self.assertEqual(first.read_bytes(), b"old fixture")
        self.protected()

    def test_opened_source_inode_survives_path_replacement(self):
        original = self.write(self.source, "auth.json", b"opened original credential")
        spec = importlib.util.spec_from_file_location("login_sync_fixture", Path(__file__).resolve().parents[1] / "src/login_sync.py")
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        fdopen = os.fdopen
        swapped = False
        def replace_before_read(descriptor, mode):
            nonlocal swapped
            if mode == "rb" and not swapped:
                original.rename(self.source / "opened-inode")
                original.write_bytes(b"replacement path credential")
                swapped = True
            return fdopen(descriptor, mode)
        with mock.patch.object(helper.os, "fdopen", side_effect=replace_before_read):
            helper.transfer(str(self.source), str(self.destination), 1024, ["auth.json"])
        self.assertEqual((self.destination / "auth.json").read_bytes(), b"opened original credential")
        self.assertEqual(original.read_bytes(), b"replacement path credential")
        self.protected()

    def test_opened_destination_parent_survives_redirection_before_publication(self):
        self.write(self.source, ".codex/auth.json", b"new anchored credential")
        self.write(self.destination, ".codex/auth.json", b"old anchored credential")
        spec = importlib.util.spec_from_file_location("login_sync_fixture", Path(__file__).resolve().parents[1] / "src/login_sync.py")
        helper = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helper)
        stage = helper.exclusive_temporary
        def redirect_after_validation(directory, content):
            (self.destination / ".codex").rename(self.destination / "held-original-parent")
            (self.destination / ".codex").symlink_to(self.outside, target_is_directory=True)
            return stage(directory, content)
        with mock.patch.object(helper, "exclusive_temporary", side_effect=redirect_after_validation):
            helper.transfer(str(self.source), str(self.destination), 1024, [".codex/auth.json"])
        self.assertEqual((self.destination / "held-original-parent/auth.json").read_bytes(), b"new anchored credential")
        self.protected()


if __name__ == "__main__":
    unittest.main()

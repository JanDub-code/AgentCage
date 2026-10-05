#!/usr/bin/env python3
"""Installer/package effects in private source, Cargo, Git and output fixtures."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import tarfile
import tempfile
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]
BINARY = None
TRIPLE = "x86_64-unknown-linux-gnu"
FILES = (".gitignore", "Cargo.toml", "Cargo.lock", "Dockerfile", "install.sh", "scripts/package.sh", "src/main.rs", "src/lib.rs",
         "src/agent.rs", "src/opencode_account.py", "src/login_sync.py", "README.md", "SECURITY.md", "LOGIN_PERSISTENCE.md", "TUTORIAL.md", "LICENSE")
FAKE_CARGO = r'''#!/usr/bin/env python3
import json, os
from pathlib import Path
import sys
args = sys.argv[1:]
with open(os.environ["CARGO_CALLS"], "a") as log:
    log.write(json.dumps({"cwd": os.getcwd(), "args": args}) + "\n")
if os.environ.get("CARGO_FAIL"):
    sys.exit(17)
target = args[args.index("--target")+1] if "--target" in args else None
directory = Path(args[args.index("--target-dir")+1] if "--target-dir" in args else os.environ.get("CARGO_TARGET_DIR", "target"))
if target:
    directory /= target
destination = directory / "release/ac"
destination.parent.mkdir(parents=True, exist_ok=True)
destination.write_text("#!/bin/sh\nprintf 'AgentCage fixture intended binary\\n'\n")
destination.chmod(0o755)
'''
FAKE_GIT = r'''#!/usr/bin/env python3
import json, os, shutil, sys
from pathlib import Path
args = sys.argv[1:]
if args and args[0] == "clone":
    destination = Path(args[-1])
    with open(os.environ["GIT_CALLS"], "a") as log:
        log.write(json.dumps({"cwd": os.getcwd(), "args": args}) + "\n")
    destination.mkdir(parents=True, exist_ok=True)
    if os.environ.get("GIT_FAIL"):
        (destination / "partial").write_text("fixture")
        sys.exit(19)
    shutil.copytree(os.environ["GIT_SOURCE"], destination, dirs_exist_ok=True)
    sys.exit(0)
if args == ["rev-parse", "HEAD"]:
    print("0123456789012345678901234567890123456789")
elif args == ["rev-parse", "--is-inside-work-tree"]:
    print("true")
elif args[:2] == ["status", "--porcelain"]:
    pass
else:
    sys.exit(1)
'''


class DistributionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-dist-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.source = self.base / "intended source"
        self.source.mkdir()
        for filename in FILES:
            source = ROOT / filename
            destination = self.source / filename
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, destination)
        self.bin = self.base / "tools"
        self.bin.mkdir()
        for name, content in (("cargo", FAKE_CARGO), ("git", FAKE_GIT),
                              ("rustc", "#!/bin/sh\nprintf 'host: x86_64-unknown-linux-gnu\\n'\n")):
            path = self.bin / name
            path.write_text(content)
            path.chmod(0o755)
        self.home = self.base / "home"
        self.home.mkdir()
        self.tmp = self.base / "temporary"
        self.tmp.mkdir()
        self.env = {"PATH": str(self.bin) + ":/usr/bin:/bin", "HOME": str(self.home), "LANG": "C.UTF-8", "TMPDIR": str(self.tmp),
                    "GIT_SOURCE": str(self.source), "GIT_CALLS": str(self.base / "git-calls"), "CARGO_CALLS": str(self.base / "cargo-calls")}

    def command(self, args, cwd=None, extra=None, input=None):
        return subprocess.run(args, cwd=cwd or self.source, env=self.env | (extra or {}), input=input,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)

    def calls(self, kind):
        file = self.base / (kind + "-calls")
        return [json.loads(line) for line in file.read_text().splitlines()] if file.exists() else []

    def installed(self, destination):
        result = self.command([str(destination / "ac"), "--help"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, b"AgentCage fixture intended binary\n")

    def test_exact_entry_points_are_executable_lf(self):
        for filename in ("install.sh", "scripts/package.sh"):
            with self.subTest(filename=filename):
                data = (self.source / filename).read_bytes()
                self.assertNotIn(b"\r", data)
                result = self.command([str(self.source / filename), "--help"])
                self.assertEqual(result.returncode, 0, result.stderr)
        for filename in ("Cargo.toml", "Cargo.lock", "Dockerfile"):
            self.assertNotIn(b"\r", (self.source / filename).read_bytes())

    def test_local_destinations_and_no_build(self):
        for arguments, destination in (([], self.home / ".local/bin"),
                                       (["--prefix", str(self.base / "prefix with spaces")], self.base / "prefix with spaces/bin"),
                                       (["--bin-dir", str(self.base / "direct with spaces")], self.base / "direct with spaces")):
            with self.subTest(arguments=arguments):
                result = self.command([str(self.source / "install.sh"), *arguments])
                self.assertEqual(result.returncode, 0, result.stderr)
                self.installed(destination)
        before = len(self.calls("cargo"))
        target = self.base / "no build bin"
        result = self.command([str(self.source / "install.sh"), "--no-build", "--bin-dir", str(target)])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(self.calls("cargo")), before)
        self.installed(target)

    def native_update_fixture(self, name):
        if BINARY is None:
            self.skipTest("native install regression needs an actual built ac: --binary PATH")
        original = BINARY.read_bytes()
        package = self.base / (name + " package with spaces")
        (package / "bin").mkdir(parents=True)
        shutil.copy2(self.source / "install.sh", package / "install.sh")
        payload = original + b"\nAgentCage native install update fixture\n"
        (package / "bin/ac").write_bytes(payload)
        (package / "bin/ac").chmod(0o755)
        (package / "RELEASE").write_text(
            "name=agentcage\nversion=0.0.1\ntarget=" + TRIPLE + "\nbinary_sha256=" +
            hashlib.sha256(payload).hexdigest() + "\n")
        destination = self.base / (name + " bin with spaces")
        destination.mkdir()
        (destination / "ac").write_bytes(original)
        (destination / "ac").chmod(0o755)
        (destination / "unrelated-sentinel").write_bytes(b"unrelated installation data\n")
        (destination / "unrelated-sentinel").chmod(0o600)
        old_help = self.command([str(destination / "ac"), "--help"])
        self.assertEqual(old_help.returncode, 0, old_help.stderr)
        self.assertGreater(len(original), 512, "a native binary must exercise the kernel file-size limit")
        return package, destination, original, payload, old_help

    def assert_native_update_preserved(self, destination, original, old_help):
        installed = (destination / "ac").read_bytes()
        self.assertEqual(len(installed), len(original))
        self.assertEqual(hashlib.sha256(installed).hexdigest(), hashlib.sha256(original).hexdigest())
        self.assertEqual(installed, original)
        self.assertEqual((destination / "ac").stat().st_mode & 0o777, 0o755)
        result = self.command([str(destination / "ac"), "--help"])
        self.assertEqual((result.returncode, result.stdout, result.stderr),
                         (old_help.returncode, old_help.stdout, old_help.stderr))
        self.assertEqual((destination / "unrelated-sentinel").read_bytes(), b"unrelated installation data\n")
        self.assertEqual((destination / "unrelated-sentinel").stat().st_mode & 0o777, 0o600)
        self.assertEqual({entry.name for entry in destination.iterdir()}, {"ac", "unrelated-sentinel"})
        self.assertEqual(list(self.tmp.iterdir()), [])

    def test_native_install_file_limit_preserves_previous_binary_and_recovers(self):
        package, destination, original, payload, old_help = self.native_update_fixture("file limit")
        self.assertEqual(shutil.which("install", path=self.env["PATH"]), "/usr/bin/install")
        arguments = [str(package / "install.sh"), "--no-build", "--bin-dir", str(destination)]
        # The limit exists only in this child shell and its native install child.
        result = self.command(["/bin/sh", "-c", 'ulimit -c 0; ulimit -f 1; exec "$@"',
                               "ac-install-file-limit", *arguments])
        self.assertEqual(result.returncode, 128 + signal.SIGXFSZ, result.stderr)
        self.assert_native_update_preserved(destination, original, old_help)
        result = self.command(arguments)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((destination / "ac").read_bytes(), payload)
        self.assertEqual((destination / "ac").stat().st_mode & 0o777, 0o755)
        self.assertEqual(self.command([str(destination / "ac"), "--help"]).stdout, old_help.stdout)
        self.assertEqual((destination / "unrelated-sentinel").read_bytes(), b"unrelated installation data\n")
        self.assertEqual((destination / "unrelated-sentinel").stat().st_mode & 0o777, 0o600)
        self.assertEqual({entry.name for entry in destination.iterdir()}, {"ac", "unrelated-sentinel"})

    def test_native_install_interruption_preserves_previous_binary_and_cleans_copy(self):
        if BINARY is None:
            self.skipTest("native install regression needs an actual built ac: --binary PATH")
        install = self.bin / "install"
        # Only a scheduling barrier is injected: /usr/bin/install performs the
        # actual copy unchanged, then its waiting shell stops before publication.
        install.write_text('#!/bin/sh\nset -eu\n/usr/bin/install "$@"\n'
                           'printf "%s\\n" "$$" > "$INSTALL_BARRIER_PID"\nkill -STOP "$$"\n')
        install.chmod(0o755)
        for sig in (signal.SIGHUP, signal.SIGINT, signal.SIGTERM):
            with self.subTest(signal=sig.name):
                package, destination, original, payload, old_help = self.native_update_fixture(sig.name)
                barrier = self.base / (sig.name + "-copy-pid")
                process = subprocess.Popen([str(package / "install.sh"), "--no-build", "--bin-dir", str(destination)],
                                           cwd=self.source, env=self.env | {"INSTALL_BARRIER_PID": str(barrier)},
                                           stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
                try:
                    deadline = time.monotonic() + 5
                    stopped = False
                    while time.monotonic() < deadline and process.poll() is None:
                        if barrier.exists():
                            child_status = Path("/proc") / barrier.read_text().strip() / "status"
                            if child_status.exists() and re.search(r"^State:\s+T", child_status.read_text(), re.M):
                                stopped = True
                                break
                        time.sleep(0.01)
                    self.assertTrue(stopped, "native install copy did not reach its scheduling barrier")
                    os.kill(process.pid, sig)
                    # Also release a foreground copy barrier, so the same
                    # behavior test reaches completion with the old installer.
                    time.sleep(0.05)
                    try:
                        os.killpg(process.pid, signal.SIGCONT)
                    except ProcessLookupError:
                        pass
                    stdout, stderr = process.communicate(timeout=5)
                    self.assertEqual(process.returncode, 128 + sig, stderr)
                    self.assertNotIn(b"installed AgentCage", stdout)
                    self.assert_native_update_preserved(destination, original, old_help)
                finally:
                    # Both the barrier process and installer are owned by this fixture.
                    try:
                        os.killpg(process.pid, signal.SIGCONT)
                        os.killpg(process.pid, signal.SIGKILL)
                    except ProcessLookupError:
                        pass
                    process.communicate(timeout=5)

    def test_stdin_bootstrap_avoids_unrelated_cargo_and_preserves_options(self):
        caller = self.base / "unrelated Cargo project"
        caller.mkdir()
        (caller / "Cargo.toml").write_text('[package]\nname="unrelated"\n')
        stale = caller / "target/release/ac"
        stale.parent.mkdir(parents=True)
        stale.write_text("#!/bin/sh\necho fixture-wrong-binary\n")
        stale.chmod(0o755)
        target = self.base / "bootstrap bin with spaces"
        result = self.command(["/bin/sh", "-s", "--", "--bin-dir", str(target)], cwd=caller,
                              input=(self.source / "install.sh").read_bytes())
        self.assertEqual(result.returncode, 0, result.stderr)
        self.installed(target)
        self.assertEqual(len(self.calls("git")), 1)
        self.assertTrue(self.calls("cargo"))
        self.assertTrue(all(Path(call["cwd"]) != caller for call in self.calls("cargo")))
        self.assertEqual(list(self.tmp.iterdir()), [])
        self.assertEqual(stale.read_text(), "#!/bin/sh\necho fixture-wrong-binary\n")
        self.assertFalse((self.home / ".local/bin/ac").exists())

    def test_bootstrap_no_build_is_explicit_failure_without_git_or_build(self):
        result = self.command(["/bin/sh", "-s", "--", "--no-build", "--prefix", str(self.base / "wanted")],
                              cwd=self.base, input=(self.source / "install.sh").read_bytes())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"--no-build", result.stderr)
        self.assertEqual(self.calls("git"), [])
        self.assertEqual(self.calls("cargo"), [])
        self.assertFalse((self.base / "wanted/bin/ac").exists())

    def test_clone_and_child_failure_cleanup_only_own_temporary_directory(self):
        sentinel = self.tmp / "unrelated-sentinel"
        sentinel.write_text("fixture safe")
        for failure in ("GIT_FAIL", "CARGO_FAIL"):
            with self.subTest(failure=failure):
                result = self.command(["/bin/sh", "-s"], cwd=self.base, extra={failure: "1"},
                                      input=(self.source / "install.sh").read_bytes())
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(list(self.tmp.iterdir()), [sentinel])
                self.assertEqual(sentinel.read_text(), "fixture safe")

    def test_external_target_and_config_override_never_use_stale_binary(self):
        config = self.source / ".cargo/config.toml"
        config.parent.mkdir()
        config.write_text('[build]\ntarget-dir="configured-target"\ntarget="aarch64-unknown-linux-gnu"\n')
        stale = self.source / "target/release/ac"
        stale.parent.mkdir(parents=True)
        stale.write_text("#!/bin/sh\necho fixture stale\n")
        stale.chmod(0o755)
        for external in (None, self.base / "external target with spaces"):
            with self.subTest(external=external):
                target = self.base / ("configured bin" if external is None else "external bin")
                extra = {"CARGO_TARGET_DIR": str(external)} if external else {}
                result = self.command([str(self.source / "install.sh"), "--bin-dir", str(target)], extra=extra)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.installed(target)
                call = self.calls("cargo")[-1]["args"]
                self.assertIn("--locked", call)
                self.assertEqual(call[call.index("--target") + 1], TRIPLE)
                self.assertEqual(Path(call[call.index("--target-dir") + 1]), external or self.source / "target")

    def package(self):
        result = self.command([str(self.source / "scripts/package.sh")],
                              extra={"CARGO_TARGET_DIR": str(self.base / "package external target")})
        self.assertEqual(result.returncode, 0, result.stderr)
        tarballs = list((self.source / "dist").glob("*.tar.gz"))
        self.assertEqual(len(tarballs), 1)
        self.assertRegex(tarballs[0].name, r"^agentcage-[0-9][0-9A-Za-z.+-]*-x86_64-unknown-linux-gnu\.tar\.gz$")
        return tarballs[0]

    def test_package_checksum_install_and_corruption_rejection(self):
        archive = self.package()
        expected = (archive.parent / (archive.name + ".sha256")).read_text().split()[0]
        self.assertEqual(expected, hashlib.sha256(archive.read_bytes()).hexdigest())
        work = self.base / "extraction with spaces"
        work.mkdir()
        shutil.copy(archive, work)
        with tarfile.open(archive) as tar:
            tar.extractall(work, filter="data")
        package = work / archive.name[:-7]
        result = self.command([str(package / "install.sh"), "--no-build", "--bin-dir", str(self.base / "package bin")], cwd=work)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.installed(self.base / "package bin")
        (package / "bin/ac").write_text("#!/bin/sh\necho fixture corrupted\n")
        result = self.command([str(package / "install.sh"), "--no-build", "--bin-dir", str(self.base / "corrupt bin")], cwd=work)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"checksum", result.stderr.lower())
        self.assertFalse((self.base / "corrupt bin/ac").exists())
        corrupt = work / "damaged.tar.gz"
        corrupt.write_bytes(archive.read_bytes() + b"fixture damage")
        check = self.command(["sha256sum", "-c"], cwd=work, input=(expected + "  damaged.tar.gz\n").encode())
        self.assertNotEqual(check.returncode, 0)

    def test_cross_target_and_invalid_version_fail_before_cargo(self):
        result = self.command([str(self.source / "scripts/package.sh"), "--target", "aarch64-unknown-linux-gnu"])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls("cargo"), [])
        manifest = self.source / "Cargo.toml"
        manifest.write_text(re.sub(r'^version\s*=.*$', 'version = "bad version"', manifest.read_text(), flags=re.M))
        result = self.command([str(self.source / "scripts/package.sh")])
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.calls("cargo"), [])

    def test_package_provenance_never_claims_a_foreign_parent_commit(self):
        git = shutil.which("git", path="/usr/bin:/bin")
        self.assertIsNotNone(git)
        (self.bin / "git").unlink()  # Use actual Git only in this owned fixture.
        def git_command(directory, *arguments):
            result = self.command([git, "-C", str(directory), *arguments])
            self.assertEqual(result.returncode, 0, result.stderr)
            return result.stdout.decode().strip()
        git_command(self.base, "init")
        git_command(self.base, "-c", "user.name=AgentCage Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "--allow-empty", "-m", "fixture parent")
        foreign = git_command(self.base, "rev-parse", "HEAD")
        def metadata():
            archive = self.package()
            with tarfile.open(archive) as package:
                return package.extractfile(archive.name[:-7] + "/RELEASE").read().decode()
        unknown = metadata()
        self.assertIn("source_revision=unknown\n", unknown)
        self.assertIn("source_state=unknown\n", unknown)
        self.assertNotIn(foreign, unknown)
        git_command(self.source, "init")
        git_command(self.source, "add", ".")
        git_command(self.source, "-c", "user.name=AgentCage Fixture", "-c", "user.email=fixture@example.invalid",
                    "commit", "-m", "fixture source")
        own = git_command(self.source, "rev-parse", "HEAD")
        known = metadata()
        self.assertIn("source_revision=" + own + "\n", known)
        self.assertIn("source_state=clean\n", known)
        (self.source / "src/untracked-fixture.rs").write_text("// fixture source input\n")
        self.assertIn("source_state=modified\n", metadata())

    def test_documented_binary_release_blocks_execute_with_archive_present(self):
        archive = self.package()
        for document in ("README.md", "TUTORIAL.md"):
            text = (ROOT / document).read_text()
            blocks = [block for block in re.findall(r"```(?:bash|sh|shell)?\n(.*?)```", text, flags=re.S)
                      if "tar " in block and "install.sh --no-build" in block]
            self.assertTrue(blocks, document + " needs an executable binary-release example")
            for block in blocks:
                with self.subTest(document=document, block=block):
                    self.assertNotIn("<", block)
                    work = Path(tempfile.mkdtemp(prefix="documented-release-", dir=self.base))
                    shutil.copy(archive, work)
                    shutil.copyfile(archive.parent / (archive.name + ".sha256"), work / (archive.name + ".sha256"))
                    # A second archive prevents an ambiguous broad wildcard from passing accidentally.
                    (work / "agentcage-0.0.0-other.tar.gz").write_bytes(b"fixture older release")
                    result = self.command(["/bin/sh", "-eu", "-c", block], cwd=work,
                                          extra={"BINDIR": str(work / "installed"), "PREFIX": str(work / "prefix")})
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertTrue((work / "installed/ac").exists() or (work / "prefix/bin/ac").exists())


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=Path, help="test an extracted exact source archive")
    parser.add_argument("--binary", type=Path, help="actual built ac for native install fault/interruption regressions")
    args, remaining = parser.parse_known_args()
    if args.source:
        ROOT = args.source.resolve()
    if args.binary:
        BINARY = args.binary.resolve()
        if not BINARY.is_file() or not os.access(BINARY, os.X_OK):
            parser.error("--binary needs an executable built ac")
    if os.name != "posix":
        parser.error("Linux is required")
    unittest.main(argv=[__file__, *remaining])

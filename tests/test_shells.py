#!/usr/bin/env python3
"""Execute documented setup and completion candidates in all three clean shells."""
import argparse
import os
from pathlib import Path
import pty
import selectors
import shlex
import shutil
import subprocess
import tempfile
import time
import unittest


AC = None
CASES = (("ac c", {"codex", "claude", "clean", "completions"}),
         ("ac completions ", {"bash", "zsh", "fish"}),
         ("ac login rm ", {"codex", "claude", "opencode", "antigravity"}),
         ("ac env rm ", {"fixture-env", "fixture_env_2"}),
         ("ac codex --env ", {"fixture-env", "fixture_env_2"}),
         ("ac shell --", {"--help", "--env"}),
         ("ac codex -", {"--", "--help", "--env", "--no-login"}),
         ("ac codex --env -- --h", {"--help"}),
         ("ac codex --env --env --h", {"--help"}),
         ("ac codex shell --", {"--", "--help", "--env", "--no-login"}),
         ("ac codex --env shell --", {"--", "--help"}),
         ("ac shell --env codex --", {"--help"}),
         ("ac env rm .", {".fixture-env_2"}),
         ("ac codex -- --", set()))
FORBIDDEN = {"ac shell --": {"--", "--no-login"},
             "ac codex --env -- --h": {"--env", "--no-login"},
             "ac codex --env --env --h": {"--env", "--no-login"},
             "ac codex --env shell --": {"--env", "--no-login"},
             "ac shell --env codex --": {"--", "--env", "--no-login"}}


class CompletionTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-shells-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        (self.bin / "ac").symlink_to(AC)
        for name in ("fixture-env", "fixture_env_2", ".fixture-env_2", "space env"):
            (self.root / ".agentcage/envs" / name / "home").mkdir(parents=True)
        self.env = {"PATH": str(self.bin) + ":/usr/bin:/bin", "HOME": str(self.root), "LANG": "C.UTF-8", "TERM": "xterm",
                    "XDG_CONFIG_HOME": str(self.root / "config"), "XDG_DATA_HOME": str(self.root / "data"),
                    "XDG_CACHE_HOME": str(self.root / "cache"), "XDG_STATE_HOME": str(self.root / "state")}

    def test_bash_clean_setup_and_candidates(self):
        self.assertIsNotNone(shutil.which("bash"), "Bash is required")
        for line, expected in CASES:
            with self.subTest(line=line):
                words = shlex.split(line) + ([""] if line.endswith(" ") else [])
                script = 'source <(ac completions bash)\nCOMP_WORDS=(' + " ".join(shlex.quote(w) for w in words) + ')\n'
                script += 'COMP_CWORD=' + str(len(words) - 1) + '\n_ac_completions\nprintf "%s\\n" "${COMPREPLY[@]}"\n'
                result = subprocess.run(["bash", "--noprofile", "--norc", "-c", script], cwd=self.root, env=self.env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, b"")
                actual = set(result.stdout.decode().splitlines()) - {""}
                self.assertNotIn("space env", actual)
                self.assertFalse(FORBIDDEN.get(line, set()) & actual, actual)
                if expected:
                    self.assertTrue(expected <= actual, actual)
                else:
                    self.assertFalse(any(value.startswith("--") for value in actual), actual)

    def test_fish_clean_setup_and_candidates(self):
        self.assertIsNotNone(shutil.which("fish"), "Fish is required, not a syntax-only substitute")
        for line, expected in CASES:
            with self.subTest(line=line):
                script = 'ac completions fish | source\ncomplete -C ' + shlex.quote(line)
                result = subprocess.run(["fish", "--no-config", "-c", script], cwd=self.root, env=self.env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=15)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stderr, b"")
                actual = {value.split("\t")[0].replace("\\ ", " ") for value in result.stdout.decode().splitlines()}
                self.assertNotIn("space env", actual)
                self.assertFalse(FORBIDDEN.get(line, set()) & actual, actual)
                if expected:
                    self.assertTrue(expected <= actual, actual)
                else:
                    self.assertFalse(any(value.startswith("--") for value in actual), actual)

    def test_zsh_documented_setup_and_native_completion_widget(self):
        self.assertIsNotNone(shutil.which("zsh"), "Zsh is required, not a syntax-only substitute")
        master, slave = pty.openpty()
        child = subprocess.Popen(["zsh", "-f"], cwd=self.root, env=self.env, stdin=slave, stdout=slave, stderr=slave,
                                 start_new_session=True)
        os.close(slave)
        self.addCleanup(os.close, master)
        def reap():
            if child.poll() is None:
                child.kill()
            child.wait(timeout=5)
        self.addCleanup(reap)
        capture = self.root / "candidates"
        script = ('source <(ac completions zsh); PS1="AC> "; '
                  '[[ "${_comps[ac]}" = _ac ]] || print __SETUP_FAILURE__; '
                  'compadd() { if [[ "$1" = -a ]]; then print -rl -- "${(@P)2}" >> ' + shlex.quote(str(capture)) + '; '
                  'elif [[ "$1" = -- ]]; then print -rl -- "${@:2}" >> ' + shlex.quote(str(capture)) + '; '
                  'else print -rl -- "$@" >> ' + shlex.quote(str(capture)) + '; fi; builtin compadd "$@"; }; '
                  '_ac_probe() { _ac; (( ac_probe_count += 1 )); print -r -- "__AC_DONE_$ac_probe_count"; }; '
                  'ac_probe_count=0; zle -C ac-probe complete-word _ac_probe; bindkey "^I" ac-probe; print __SETUP_READY__\n')
        os.write(master, script.encode())
        setup = self.read_until(master, b"__SETUP_READY__", 15)
        self.assertNotIn(b"__SETUP_FAILURE__\r\n", setup)
        for index, (line, expected) in enumerate(CASES, 1):
            with self.subTest(line=line):
                capture.write_text("")
                os.write(master, b"\x15" + line.encode() + b"\t")
                output = self.read_until(master, ("__AC_DONE_" + str(index)).encode(), 10)
                self.assertNotIn(b"command not found", output)
                self.assertNotIn(b"can only be called from completion", output)
                actual = set(capture.read_text().splitlines()) - {""}
                self.assertNotIn("space env", actual)
                self.assertFalse(FORBIDDEN.get(line, set()) & actual, actual)
                if expected:
                    self.assertTrue(expected <= actual, actual)
                else:
                    self.assertFalse(any(value.startswith("--") for value in actual), actual)
        os.write(master, b"\x15exit\n")
        self.assertEqual(child.wait(timeout=5), 0)

    def read_until(self, descriptor, marker, timeout):
        deadline = time.monotonic() + timeout
        output = b""
        with selectors.DefaultSelector() as selector:
            selector.register(descriptor, selectors.EVENT_READ)
            while marker not in output:
                remaining = deadline - time.monotonic()
                self.assertGreater(remaining, 0, repr(output))
                self.assertTrue(selector.select(remaining), repr(output))
                try:
                    data = os.read(descriptor, 65536)
                except OSError:
                    self.fail(repr(output))
                self.assertTrue(data, repr(output))
                output += data
        return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args, remaining = parser.parse_known_args()
    with tempfile.TemporaryDirectory(prefix="ac-shell-binary-") as binary_dir:
        AC = Path(binary_dir) / "ac"
        shutil.copy2(args.binary.resolve(), AC)
        unittest.main(argv=[__file__, *remaining])

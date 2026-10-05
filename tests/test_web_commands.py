#!/usr/bin/env python3
"""Execute the published shell commands with harmless private fixtures."""
from html.parser import HTMLParser
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[1]
URL = "https://github.com/JanDub-code/AgentCage.git"


class Commands(HTMLParser):
    def __init__(self, source):
        super().__init__(convert_charrefs=True)
        self.commands = {}
        self.current = None
        self.feed(source)

    def handle_starttag(self, tag, attrs):
        identifier = dict(attrs).get("id")
        if tag == "code" and identifier:
            self.current = identifier
            self.commands[identifier] = ""

    def handle_data(self, text):
        if self.current:
            self.commands[self.current] += text

    def handle_endtag(self, tag):
        if tag == "code":
            self.current = None


def markdown_command(filename, heading, index=0):
    source = (ROOT / filename).read_text()
    section = source.split(heading, 1)[1].split("\n## ", 1)[0]
    return re.findall(r"```bash\n(.*?)\n```", section, re.S)[index]


@unittest.skipUnless(os.name == "posix", "published commands require a POSIX shell")
class WebCommandTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-web-commands-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.home = self.base / "private home"
        self.cwd = self.base / "fallback cwd"
        self.project = self.base / "intended project"
        self.tools = self.base / "tools"
        for directory in (self.home, self.cwd, self.project, self.tools):
            directory.mkdir()
        self.calls = self.base / "calls"
        self.install_calls = self.base / "install calls"
        self.install_arguments = self.base / "install arguments"
        self.download_calls = self.base / "download calls"
        self.env = {"PATH": str(self.tools) + ":/usr/bin:/bin", "HOME": str(self.home),
                    "LANG": "C.UTF-8", "AC_CALLS": str(self.calls),
                    "AC_INSTALL_CALLS": str(self.install_calls), "AC_INSTALL_ARGUMENTS": str(self.install_arguments),
                    "AC_DOWNLOAD_CALLS": str(self.download_calls)}
        self.html = Commands((ROOT / "docs/index.html").read_text()).commands
        self.binary = self.base / "binary"
        self.write_executable(self.binary, """#!/bin/sh
printf '%s|%s\\n' "$PWD" "$*" >> "$AC_CALLS"
if [ "$#" -eq 0 ]; then exit "${AC_INIT_STATUS:-0}"; fi
printf 'AgentCage intended fixture binary\\n'
""")

    def write_executable(self, path, contents):
        path.write_text(contents)
        path.chmod(0o755)

    def run_shell(self, script, extra=None):
        return subprocess.run(["/bin/sh", "-c", script], cwd=self.cwd,
                              env=self.env | (extra or {}), capture_output=True, timeout=10)

    def logged_calls(self):
        return [line.split("|", 1) for line in self.calls.read_text().splitlines()] if self.calls.exists() else []

    def quick_starts(self):
        return {"HTML": (self.html["cmd-start"], "/path/to/project", "codex"),
                "README": (markdown_command("README.md", "## Quick start"), "/path/to/project", "codex"),
                "tutorial": (markdown_command("TUTORIAL.md", "## 4. Initialize the right project"), "/path/to/project", "codex"),
                "SECURITY": (markdown_command("SECURITY.md", "## Recommended usage"), "/path/to/throwaway-project", "codex --no-login"),
                "init": (markdown_command("init.md", "Leave the installer checkout"), "/path/to/project", "shell"),
                "Markdown site": (markdown_command("docs/index.md", "## Quick start"), "/path/to/project", "codex")}

    def test_failed_cd_never_runs_init_in_the_fallback_directory(self):
        self.write_executable(self.tools / "ac", self.binary.read_text())
        missing = self.base / "missing project"
        for source, (script, example, command) in self.quick_starts().items():
            with self.subTest(source=source):
                self.calls.unlink(missing_ok=True)
                result = self.run_shell(script.replace(example, shlex.quote(str(missing))))
                self.assertNotEqual(result.returncode, 0, result)
                self.assertEqual(self.logged_calls(), [], result.stderr)

    def test_failed_init_never_starts_the_agent(self):
        self.write_executable(self.tools / "ac", self.binary.read_text())
        for source, (script, example, command) in self.quick_starts().items():
            with self.subTest(source=source):
                self.calls.unlink(missing_ok=True)
                result = self.run_shell(script.replace(example, shlex.quote(str(self.project))),
                                        {"AC_INIT_STATUS": "17"})
                self.assertEqual(result.returncode, 17, result.stderr)
                self.assertEqual(self.logged_calls(), [[str(self.project), ""]])

    def test_successful_start_runs_init_then_one_agent_in_the_project(self):
        self.write_executable(self.tools / "ac", self.binary.read_text())
        for source, (script, example, command) in self.quick_starts().items():
            with self.subTest(source=source):
                self.calls.unlink(missing_ok=True)
                result = self.run_shell(script.replace(example, shlex.quote(str(self.project))))
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.logged_calls(), [[str(self.project), ""], [str(self.project), command]])

    def path_commands(self):
        return {"HTML": self.html["cmd-path"],
                "README": markdown_command("README.md", "### Verify PATH"),
                "tutorial": markdown_command("TUTORIAL.md", "## 3. Verify PATH")}

    def test_path_finds_script_and_default_or_custom_cargo_installations(self):
        for installation in ("script", "cargo", "custom Cargo home"):
            extra = {"CARGO_HOME": str(self.base / "custom cargo home")} if installation.startswith("custom") else {}
            directory = self.home / ".local/bin" if installation == "script" else (
                Path(extra["CARGO_HOME"]) / "bin" if extra else self.home / ".cargo/bin")
            directory.mkdir(parents=True)
            self.write_executable(directory / "ac", self.binary.read_text())
            for source, script in self.path_commands().items():
                with self.subTest(installation=installation, source=source):
                    self.calls.unlink(missing_ok=True)
                    result = self.run_shell(script, extra)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(b"AgentCage intended fixture binary", result.stdout)
                    self.assertEqual(self.logged_calls(), [[str(self.cwd), "--help"]])
            (directory / "ac").unlink()

    def installation_fixtures(self):
        installer = self.base / "installer"
        self.write_executable(installer, """#!/bin/sh
printf '%s|%s\\n' "$PWD" "$*" >> "$AC_INSTALL_CALLS"
printf '%s\\n' "$#" "$@" > "$AC_INSTALL_ARGUMENTS"
[ "${AC_INSTALL_STATUS:-0}" = 0 ] || exit "$AC_INSTALL_STATUS"
destination="$HOME/.local/bin"
if [ "${1:-}" = cargo ]; then destination="${CARGO_HOME:-$HOME/.cargo}/bin"; shift; fi
if [ "$#" -gt 0 ]; then
    [ "$#" -eq 2 ] && [ "$1" = --bin-dir ] || exit 94
    destination="$2"
fi
mkdir -p "$destination"
cp "$AC_BINARY" "$destination/ac"
chmod +x "$destination/ac"
""")
        self.write_executable(self.cwd / "install.sh", installer.read_text())
        self.env.update(AC_INSTALLER=str(installer), AC_BINARY=str(self.binary))
        self.write_executable(self.tools / "curl", """#!/bin/sh
[ "$*" = '-fsSL https://raw.githubusercontent.com/JanDub-code/AgentCage/main/install.sh' ] || exit 91
printf '%s\\n' "$*" >> "$AC_DOWNLOAD_CALLS"
if [ "${AC_CURL_BODY:-installer}" != empty ]; then cat "$AC_INSTALLER"; fi
exit "${AC_CURL_STATUS:-0}"
""")
        self.write_executable(self.tools / "cargo", f"""#!/bin/sh
[ "$*" = 'install --git {URL} --locked' ] || exit 92
"$AC_INSTALLER" cargo
""")
        self.write_executable(self.tools / "git", f"""#!/bin/sh
[ "$*" = 'clone {URL} AgentCage' ] || exit 93
[ "${{AC_GIT_STATUS:-0}}" = 0 ] || exit "$AC_GIT_STATUS"
[ "${{AC_GIT_NO_CHECKOUT:-0}}" = 0 ] || exit 0
mkdir -p "$3"
cp "$AC_INSTALLER" "$3/install.sh"
chmod +x "$3/install.sh"
""")

    def curl_commands(self):
        return {"HTML": self.html["cmd-curl"],
                "README default": markdown_command("README.md", "### Quick installation via curl"),
                "README custom": markdown_command("README.md", "### Quick installation via curl", 1)}

    def source_commands(self):
        return {"HTML": self.html["cmd-git"],
                "README": markdown_command("README.md", "### From source"),
                "tutorial": markdown_command("TUTORIAL.md", "### From source"),
                "Markdown site": markdown_command("docs/index.md", "## Install")}

    def local_commands(self):
        return {"init": markdown_command("init.md", "# Quick local install"),
                "tutorial reinstall": markdown_command("TUTORIAL.md", "To build and install the current checkout again:")}

    def installation_commands(self):
        scripts = {}
        for kind, commands in (("source", self.source_commands()), ("local", self.local_commands()), ("curl", self.curl_commands())):
            scripts.update({kind + " " + name: script for name, script in commands.items()})
        scripts["HTML Cargo"] = self.html["cmd-cargo"]
        scripts["README Cargo"] = markdown_command("README.md", "Cargo can also install the repository directly:")
        return scripts

    def reset_installation(self):
        for record in (self.calls, self.install_calls, self.install_arguments, self.download_calls):
            record.unlink(missing_ok=True)
        for directory in (self.home / ".local/bin", self.home / ".cargo/bin", self.base / "custom cargo home/bin", self.home / "custom install bin"):
            (directory / "ac").unlink(missing_ok=True)
        checkout = self.cwd / "AgentCage"
        if checkout.exists():
            shutil.rmtree(checkout)

    def installer_calls(self):
        return self.install_calls.read_text().splitlines() if self.install_calls.exists() else []

    def test_failed_empty_or_partial_download_never_executes_an_installer(self):
        self.installation_fixtures()
        for source, script in self.curl_commands().items():
            for body in ("empty", "installer"):
                with self.subTest(source=source, body=body):
                    self.reset_installation()
                    result = self.run_shell(script, {"AC_CURL_STATUS": "22", "AC_CURL_BODY": body})
                    installed = [str(path) for path in self.home.rglob("ac") if path.is_file()]
                    self.assertEqual((result.returncode, self.installer_calls(), installed), (22, [], []), result.stderr)
                    self.assertEqual(len(self.download_calls.read_text().splitlines()), 1)

    def test_successful_custom_curl_preserves_the_directory_argument(self):
        self.installation_fixtures()
        published = self.curl_commands()["README custom"]
        script = published.replace('"$HOME/.local/bin"', '"$HOME/custom install bin"')
        result = self.run_shell(script)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.install_arguments.read_text().splitlines(), ["2", "--bin-dir", str(self.home / "custom install bin")])
        self.assertTrue((self.home / "custom install bin/ac").is_file())
        self.assertFalse((self.home / ".local/bin/ac").exists())

    def test_successful_empty_download_reports_error_and_stops_followups(self):
        self.installation_fixtures()
        self.write_executable(self.tools / "ac", self.binary.read_text())
        path_result = self.base / "path after empty download"
        for source, script in self.curl_commands().items():
            with self.subTest(source=source):
                self.reset_installation()
                flow = script.rstrip() + ' &&\nexport PATH="$HOME/.local/bin:$PATH" &&\nac --help'
                result = self.run_shell("trap 'printf \"%s\\n\" \"$PATH\" > \"$AC_PATH_RESULT\"' EXIT\n" + flow,
                                        {"AC_CURL_STATUS": "0", "AC_CURL_BODY": "empty",
                                         "AC_PATH_RESULT": str(path_result)})
                installed = [str(path) for path in self.home.rglob("ac") if path.is_file()]
                self.assertEqual((result.returncode != 0, self.installer_calls(), installed,
                                  self.logged_calls(), path_result.read_text().strip(),
                                  b"empty" in result.stderr.lower()),
                                 (True, [], [], [], self.env["PATH"], True), result.stderr)
                self.assertEqual(len(self.download_calls.read_text().splitlines()), 1)

    def test_failed_installer_does_not_continue_path_or_run_an_existing_ac(self):
        self.installation_fixtures()
        self.write_executable(self.tools / "ac", self.binary.read_text())
        path_result = self.base / "path after failure"
        for source, script in self.installation_commands().items():
            with self.subTest(source=source):
                self.reset_installation()
                result = self.run_shell("trap 'printf \"%s\\n\" \"$PATH\" > \"$AC_PATH_RESULT\"' EXIT\n" + script,
                                        {"AC_INSTALL_STATUS": "23", "AC_PATH_RESULT": str(path_result)})
                self.assertEqual(result.returncode, 23, result.stderr)
                self.assertEqual(self.logged_calls(), [], result.stderr)
                self.assertEqual(path_result.read_text().strip(), self.env["PATH"])
                self.assertEqual(len(self.installer_calls()), 1)

    def test_successful_markdown_installation_and_verification_flows(self):
        self.installation_fixtures()
        for source, script in self.installation_commands().items():
            with self.subTest(source=source):
                self.reset_installation()
                result = self.run_shell(script)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(len(self.installer_calls()), 1)
                destination = self.home / (".cargo/bin/ac" if source in ("HTML Cargo", "README Cargo") else ".local/bin/ac")
                self.assertTrue(destination.is_file())
                if source.startswith("local ") or source == "source Markdown site":
                    self.assertIn(b"AgentCage intended fixture binary", result.stdout)

    def test_every_html_installation_reaches_its_binary_using_the_published_path(self):
        self.installation_fixtures()
        for command in ("cmd-curl", "cmd-cargo", "cmd-git"):
            for custom_home in (False, True):
                with self.subTest(command=command, custom_cargo_home=custom_home):
                    extra = {"CARGO_HOME": str(self.base / "custom cargo home")} if custom_home else {}
                    result = self.run_shell(self.html[command] + "\n" + self.html["cmd-path"], extra)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn(b"AgentCage intended fixture binary", result.stdout)
                    if command == "cmd-cargo":
                        cargo_home = Path(extra["CARGO_HOME"]) if extra else self.home / ".cargo"
                        destination = cargo_home / "bin/ac"
                    else:
                        destination = self.home / ".local/bin/ac"
                    self.assertTrue(destination.is_file())
                    destination.unlink()
                    # The Git fixture deliberately supports a second independent clone.
                    if command == "cmd-git":
                        (self.cwd / "AgentCage/install.sh").unlink()
                        (self.cwd / "AgentCage").rmdir()

    def test_failed_source_clone_never_runs_an_existing_checkout_installer(self):
        self.installation_fixtures()
        checkout = self.cwd / "AgentCage"
        checkout.mkdir()
        self.write_executable(checkout / "install.sh", '#!/bin/sh\nprintf "wrong checkout\\n" >> "$AC_CALLS"\n')
        for source, script in self.source_commands().items():
            with self.subTest(source=source):
                result = self.run_shell(script, {"AC_GIT_STATUS": "19"})
                self.assertEqual(result.returncode, 19, result.stderr)
                self.assertEqual(self.logged_calls(), [])

    def test_failed_source_cd_never_runs_the_fallback_directory_installer(self):
        self.installation_fixtures()
        for source, script in self.source_commands().items():
            with self.subTest(source=source):
                self.reset_installation()
                result = self.run_shell(script, {"AC_GIT_NO_CHECKOUT": "1"})
                self.assertNotEqual(result.returncode, 0, result.stderr)
                self.assertEqual(self.installer_calls(), [], result.stderr)
                self.assertEqual(self.logged_calls(), [], result.stderr)


if __name__ == "__main__":
    unittest.main()

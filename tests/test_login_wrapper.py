#!/usr/bin/env python3
"""Actual embedded login wrapper plus Rust finalization, with owned fixtures."""
import argparse
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import tempfile
import unittest

from test_opencode_account import DOCUMENT, ROW, SCHEMA


ROOT = Path(__file__).resolve().parents[1]
AC = None
OLD = b'{"OPENAI_API_KEY":"fixture-original"}'
NEW = b'{"OPENAI_API_KEY":"fixture-updated"}'
OUTSIDE = b"fixture-outside-sentinel"
SYNTHETIC_JWT = "e30.e30.Zml4dHVyZQ"
CODEX_OAUTH = {"tokens": {"id_token": SYNTHETIC_JWT, "access_token": SYNTHETIC_JWT,
                           "refresh_token": "fixture-refresh", "account_id": None}}
CLAUDE_OAUTH = {"claudeAiOauth": {"accessToken": "fixture-original-access",
                                 "refreshToken": "fixture-original-refresh",
                                 "expiresAt": 1000, "scopes": ["user:inference"]}}


def document_bytes(value):
    return json.dumps(value, separators=(",", ":")).encode()


# Shapes from pinned Codex 0.160.0 AuthDotJson/TokenData; these values are synthetic.
INVALID_CODEX = {
    "malformed JSON": b'{"OPENAI_API_KEY":',
    "false document": b"false",
    "empty file": b"",
    "non UTF-8": b"\xff\xfe fixture",
    "tokens number": b'{"tokens":42}',
    "API key boolean": b'{"OPENAI_API_KEY":false}',
    "API key number": b'{"OPENAI_API_KEY":42}',
    "API key array": b'{"OPENAI_API_KEY":["fixture"]}',
    "empty tokens object": b'{"tokens":{}}',
    "incomplete tokens-only": b'{"tokens":{"access_token":"fixture-only"}}',
}
for field, invalid_value in (("id_token", False), ("access_token", False),
                             ("refresh_token", 42), ("account_id", 42)):
    INVALID_CODEX["wrong token field " + field] = document_bytes(
        {"tokens": CODEX_OAUTH["tokens"] | {field: invalid_value}})

INVALID_CLAUDE_OAUTH = {"empty OAuth object": b'{"claudeAiOauth":{}}'}
for description, field, invalid_value in (
        ("access boolean", "accessToken", False),
        ("refresh number", "refreshToken", 42),
        ("expiry string", "expiresAt", "1000"),
        ("expiry negative", "expiresAt", -1),
        ("expiry fraction", "expiresAt", 0.5),
        ("refresh expiry string", "refreshTokenExpiresAt", "1000"),
        ("refresh expiry negative", "refreshTokenExpiresAt", -1),
        ("refresh expiry fraction", "refreshTokenExpiresAt", 0.5),
        ("scopes string", "scopes", "user:inference"),
        ("scopes boolean member", "scopes", ["user:inference", False]),
        ("client ID number", "clientId", 42),
        ("subscription boolean", "subscriptionType", False),
        ("rate limit array", "rateLimitTier", [])):
    INVALID_CLAUDE_OAUTH[description] = document_bytes(
        {"claudeAiOauth": CLAUDE_OAUTH["claudeAiOauth"] | {field: invalid_value}})
INVALID_CLAUDE_OAUTH["non-finite expiry"] = b'{"claudeAiOauth":{"accessToken":"fixture","expiresAt":1e999}}'
INVALID_CLAUDE_OAUTH["non-finite refresh expiry"] = b'{"claudeAiOauth":{"accessToken":"fixture","refreshTokenExpiresAt":1e999}}'


# The host parser and the binary's embedded bridge must accept the same typed
# auth-only account document. These credentials and endpoints are synthetic.
EMPTY_OPENCODE_ACCOUNT = {"accounts": [], "active_account_id": None, "active_org_id": None}
VALID_OPENCODE_ACCOUNTS = {
    "one account": DOCUMENT,
    "no selection": DOCUMENT | {"active_account_id": None, "active_org_id": None},
    "nullable expiry": DOCUMENT | {"accounts": [ROW | {"token_expiry": None}]},
    "maximum expiry": DOCUMENT | {"accounts": [ROW | {"token_expiry": 2**63 - 1}]},
    "multiple Unicode accounts": DOCUMENT | {"accounts": [ROW, ROW | {
        "id": 'fixture druhý účet " ☃', "email": "druhy☃@example.invalid", "token_expiry": None}]},
    "logout": EMPTY_OPENCODE_ACCOUNT,
}
INVALID_OPENCODE_ACCOUNTS = {
    "empty root": b"{}",
    "shallow empty account": b'{"accounts":[{}]}',
    "shallow ID-only account": b'{"accounts":[{"id":"fixture-account"}]}',
    "nested account key": b'{"nested":{"accounts":[{"id":"fixture-account"}]}}',
    "incomplete empty document": b'{"accounts":[]}',
    "malformed JSON": b'{"accounts":[',
    "extra root field": document_bytes(DOCUMENT | {"extra": "fixture"}),
    "duplicate account IDs": document_bytes(DOCUMENT | {"accounts": [ROW, ROW]}),
    "extra account field": document_bytes(DOCUMENT | {"accounts": [ROW | {"extra": "fixture"}]}),
    "unknown active ID": document_bytes(DOCUMENT | {"active_account_id": "fixture-missing"}),
    "empty active ID": document_bytes(DOCUMENT | {"active_account_id": ""}),
    "boolean active ID": document_bytes(DOCUMENT | {"active_account_id": False}),
    "organization without active account": document_bytes(DOCUMENT | {"active_account_id": None}),
    "empty organization": document_bytes(DOCUMENT | {"active_org_id": ""}),
    "boolean organization": document_bytes(DOCUMENT | {"active_org_id": False}),
    "too many accounts": document_bytes(EMPTY_OPENCODE_ACCOUNT | {"accounts": [
        ROW | {"id": "fixture-" + str(index)} for index in range(10001)]}),
}
for field in DOCUMENT:
    INVALID_OPENCODE_ACCOUNTS["missing root field " + field] = document_bytes(
        {key: value for key, value in DOCUMENT.items() if key != field})
for description, value in (("object", {}), ("null", None), ("boolean", False),
                           ("string", "fixture"), ("number", 42)):
    INVALID_OPENCODE_ACCOUNTS["accounts " + description] = document_bytes(DOCUMENT | {"accounts": value})
for description, value in (("null", None), ("boolean", False), ("string", "fixture")):
    INVALID_OPENCODE_ACCOUNTS["account row " + description] = document_bytes(DOCUMENT | {"accounts": [value]})
for field in ROW:
    INVALID_OPENCODE_ACCOUNTS["missing account field " + field] = document_bytes(
        DOCUMENT | {"accounts": [{key: value for key, value in ROW.items() if key != field}]})
for field in ("id", "email", "url", "access_token", "refresh_token"):
    for description, value in (("empty", ""), ("boolean", False)):
        INVALID_OPENCODE_ACCOUNTS[field + " " + description] = document_bytes(
            DOCUMENT | {"accounts": [ROW | {field: value}]})
for description, value in (("boolean", True), ("negative", -1), ("overflow", 2**63),
                           ("float", 1.0), ("fraction", 1.5), ("string", "soon")):
    INVALID_OPENCODE_ACCOUNTS["expiry " + description] = document_bytes(
        DOCUMENT | {"accounts": [ROW | {"token_expiry": value}]})


class WrapperTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-wrapper-")
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.project = self.base / "project"
        self.project.mkdir()
        self.home = self.base / "host-home"
        self.home.mkdir()
        self.container = self.base / "container-home"
        self.container.mkdir(mode=0o700)
        self.tmp = self.base / "tmp"
        self.tmp.mkdir()
        self.outside = self.base / "outside"
        self.outside.mkdir()
        (self.outside / "auth.json").write_bytes(OUTSIDE)
        self.store = self.home / "data/agentcage/logins/codex"
        (self.store / ".codex").mkdir(parents=True, mode=0o700)
        (self.store / ".codex/auth.json").write_bytes(OLD)
        (self.store / "auth.json").write_bytes(OLD)
        self.tools = self.base / "bin"
        self.tools.mkdir()
        shutil.copyfile(ROOT / "tests/fake_podman.py", self.tools / "podman")
        (self.tools / "podman").chmod(0o755)
        agent = self.tools / "codex"
        agent.write_text("""#!/usr/bin/env python3
import errno, os, pathlib, shutil, sys
home=pathlib.Path(os.environ['HOME'])
path=home/'.codex/auth.json'
assert path.read_bytes()==b'{"OPENAI_API_KEY":"fixture-original"}'
action=os.environ['FIXTURE_ACTION']
if action=='update':
    path.write_bytes(b'{"OPENAI_API_KEY":"fixture-updated"}')
elif action=='logout':
    path.unlink()
elif action in ('parent', 'dangling', 'child42'):
    (home/'auth.json').write_bytes(b'{"OPENAI_API_KEY":"fixture-updated"}')
    shutil.rmtree(home/'.codex')
    (home/'.codex').symlink_to(os.environ['FIXTURE_PARENT'], target_is_directory=True)
elif action=='leaf':
    path.unlink(); path.symlink_to(pathlib.Path(os.environ['FIXTURE_PARENT'])/'auth.json')
elif action=='fifo':
    path.unlink(); os.mkfifo(path, 0o600)
elif action=='hardlink':
    path.unlink(); os.link(pathlib.Path(os.environ['FIXTURE_PARENT'])/'auth.json',path)
elif action=='oversize':
    path.write_bytes(b'x'*(10*1024*1024+1))
elif action in ('export-denied', 'export-denied42'):
    # Remove both imported files in this disposable HOME, then deny the
    # second export unlink in this run's own staging directory. The first
    # allowlisted deletion remains possible, exposing a partial export.
    (home/'auth.json').unlink()
    path.unlink()
    stages=list(pathlib.Path(os.environ['TMPDIR']).glob('agentcage-login-codex-*'))
    assert len(stages)==1, stages
    blocked=stages[0]/'.codex'
    blocked.chmod(0o500)
    try:
        (blocked/'auth.json').unlink()
    except PermissionError as error:
        assert error.errno==errno.EACCES, error
    else:
        raise AssertionError('fixture did not obtain real kernel EACCES')
else: raise AssertionError(action)
print('fixture actual agent output')
sys.exit(42 if action in ('child42', 'export-denied42') else 0)
""")
        agent.chmod(0o755)
        fixture = self.base / "podman"
        (fixture / "calls").mkdir(parents=True)
        (fixture / "state").write_text("{}")
        self.env = {"PATH": str(self.tools) + ":/usr/bin:/bin", "HOME": str(self.home), "LANG": "C.UTF-8",
                    "XDG_DATA_HOME": str(self.home / "data"), "TMPDIR": str(self.tmp),
                    "FAKE_PODMAN_ROOT": str(fixture), "FAKE_EXECUTE_WRAPPER": "1",
                    "FAKE_CONTAINER_HOME": str(self.container)}
        result = self.command([])
        self.assertEqual(result.returncode, 0, result.stderr)

    def command(self, arguments, action=None):
        parent = self.base / "absent-parent" if action == "dangling" else self.outside
        environment = self.env | {"FIXTURE_ACTION": action or "update", "FIXTURE_PARENT": str(parent)}
        return subprocess.run([str(AC), *arguments], cwd=self.project, env=environment,
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)

    def assert_untouched(self):
        self.assertEqual((self.store / ".codex/auth.json").read_bytes(), OLD)
        self.assertEqual((self.store / "auth.json").read_bytes(), OLD)
        self.assertEqual((self.outside / "auth.json").read_bytes(), OUTSIDE)
        self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_wrapper_roundtrip_and_private_file(self):
        result = self.command(["codex"], "update")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout, b"fixture actual agent output\n")
        self.assertEqual((self.store / ".codex/auth.json").read_bytes(), NEW)
        self.assertEqual((self.store / ".codex/auth.json").stat().st_mode & 0o777, 0o600)
        self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_wrapper_genuine_logout(self):
        result = self.command(["codex"], "logout")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.store / ".codex/auth.json").exists())
        self.assertEqual((self.store / "auth.json").read_bytes(), OLD)
        self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_wrapper_parent_redirection_preserves_entire_snapshot(self):
        result = self.command(["codex"], "parent")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b"failed to save login", result.stderr)
        self.assert_untouched()

    def test_real_wrapper_dangling_parent_is_not_logout(self):
        result = self.command(["codex"], "dangling")
        self.assertNotEqual(result.returncode, 0)
        self.assert_untouched()

    def test_real_wrapper_rejects_leaf_and_special_files(self):
        for action in ("leaf", "fifo", "hardlink", "oversize"):
            with self.subTest(action=action):
                # Reset only this test's disposable container home.
                shutil.rmtree(self.container)
                self.container.mkdir(mode=0o700)
                (self.store / ".codex/auth.json").write_bytes(OLD)
                (self.store / "auth.json").write_bytes(OLD)
                result = self.command(["codex"], action)
                self.assertNotEqual(result.returncode, 0)
                self.assert_untouched()

    def test_real_wrapper_child_failure_survives_copy_out_error(self):
        result = self.command(["codex"], "child42")
        self.assertEqual(result.returncode, 42, result.stderr)
        self.assertIn(b"failed to save login", result.stderr)
        self.assert_untouched()

    def test_real_wrapper_kernel_eacces_during_export_preserves_entire_host_snapshot(self):
        if os.geteuid() == 0:
            self.skipTest("real permission denial requires an unprivileged fixture UID")
        for child_exit in (0, 42):
            with self.subTest(child_exit=child_exit):
                shutil.rmtree(self.container)
                self.container.mkdir(mode=0o700)
                for relative in ("auth.json", ".codex/auth.json"):
                    path = self.store / relative
                    path.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    path.write_bytes(OLD)
                try:
                    result = self.command(["codex"], "export-denied42" if child_exit else "export-denied")
                    self.assertEqual(result.stdout, b"fixture actual agent output\n", result.stderr)
                    self.assertEqual(result.returncode, child_exit or 1, result.stderr)
                    stages = list(self.tmp.glob("agentcage-login-codex-*"))
                    self.assertEqual(len(stages), 1, result.stderr)
                    stage = stages[0]
                    # The actual helper completed the first unlink, but the
                    # kernel denied the second. Cleanup cannot remove that
                    # readonly parent and must report the remaining own path.
                    self.assertFalse((stage / "auth.json").exists())
                    self.assertEqual((stage / ".codex/auth.json").read_bytes(), OLD)
                    self.assertIn(b"cannot remove login staging", result.stderr)
                    self.assertIn(os.fsencode(stage), result.stderr)
                    self.assertEqual((self.store / "auth.json").read_bytes()
                                     if (self.store / "auth.json").exists() else None, OLD, result.stderr)
                    self.assertEqual((self.store / ".codex/auth.json").read_bytes(), OLD)
                    self.assertEqual((self.outside / "auth.json").read_bytes(), OUTSIDE)
                    self.assertNotIn(b"fixture-original", result.stderr)
                finally:
                    # Restore permissions only on this test's disposable
                    # leftovers so teardown works after either binary result.
                    for stage in self.tmp.glob("agentcage-login-codex-*"):
                        blocked = stage / ".codex"
                        if blocked.is_dir():
                            blocked.chmod(0o700)
                        shutil.rmtree(stage)

    def write_auth_fixture(self, agent, relative):
        fixture = self.tools / agent
        fixture.write_text("""#!/usr/bin/env python3
import os,pathlib,sys
home=pathlib.Path(os.environ['HOME'])
path=home/%r
assert path.read_bytes()==bytes.fromhex(os.environ['FIXTURE_AUTH_ORIGINAL_HEX'])
if os.environ.get('FIXTURE_AUTH_DELETE')=='1':
    path.unlink()
else:
    path.write_bytes(bytes.fromhex(os.environ['FIXTURE_AUTH_UPDATED_HEX']))
if 'FIXTURE_OTHER_REL' in os.environ:
    (home/os.environ['FIXTURE_OTHER_REL']).write_bytes(bytes.fromhex(os.environ['FIXTURE_OTHER_HEX']))
print('fixture actual credential agent output')
sys.exit(int(os.environ.get('FIXTURE_AUTH_EXIT','0')))
""" % relative)
        fixture.chmod(0o755)

    def auth_command(self, agent, original, updated, child_exit=0, other=None):
        # Reset only this run's disposable container HOME before each import.
        shutil.rmtree(self.container)
        self.container.mkdir(mode=0o700)
        extra = {"FIXTURE_AUTH_ORIGINAL_HEX": original.hex(), "FIXTURE_AUTH_EXIT": str(child_exit)}
        if updated is None:
            extra["FIXTURE_AUTH_DELETE"] = "1"
        else:
            extra["FIXTURE_AUTH_UPDATED_HEX"] = updated.hex()
        if other:
            relative, value = other
            extra.update(FIXTURE_OTHER_REL=relative, FIXTURE_OTHER_HEX=value.hex())
        result = subprocess.run([str(AC), agent], cwd=self.project, env=self.env | extra,
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(result.stdout, b"fixture actual credential agent output\n", result.stderr)
        return result

    def test_real_codex_wrapper_invalid_auth_dot_json_preserves_entire_snapshot(self):
        self.write_auth_fixture("codex", ".codex/auth.json")
        for description, invalid in INVALID_CODEX.items():
            for child_exit in (0, 42):
                with self.subTest(content=description, child_exit=child_exit):
                    for relative in ("auth.json", ".codex/auth.json"):
                        (self.store / relative).write_bytes(OLD)
                    result = self.auth_command("codex", OLD, invalid, child_exit, other=("auth.json", NEW))
                    self.assertEqual((result.returncode, (self.store / "auth.json").read_bytes(),
                                      (self.store / ".codex/auth.json").read_bytes(),
                                      list(self.tmp.glob("agentcage-login-*"))),
                                     (1 if child_exit == 0 else 42, OLD, OLD, []), result.stderr)
                    self.assertIn(b"failed to save login", result.stderr)
                    self.assertNotIn(b"fixture-original", result.stderr)

    def test_real_codex_wrapper_oauth_token_data_roundtrip(self):
        self.write_auth_fixture("codex", ".codex/auth.json")
        for account_id in ("omitted", None):
            updated = {"tokens": dict(CODEX_OAUTH["tokens"])}
            if account_id == "omitted":
                del updated["tokens"]["account_id"]
            encoded = document_bytes(updated)
            for child_exit in (0, 42):
                with self.subTest(account_id=account_id, child_exit=child_exit):
                    (self.store / ".codex/auth.json").write_bytes(OLD)
                    result = self.auth_command("codex", OLD, encoded, child_exit)
                    self.assertEqual(result.returncode, child_exit, result.stderr)
                    self.assertEqual((self.store / ".codex/auth.json").read_bytes(), encoded)
                    self.assertEqual((self.store / "auth.json").read_bytes(), OLD)
                    self.assertEqual((self.store / ".codex/auth.json").stat().st_mode & 0o777, 0o600)
                    self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def claude_oauth_store(self):
        store = self.home / "data/agentcage/logins/claude"
        (store / ".claude").mkdir(parents=True, mode=0o700)
        self.write_auth_fixture("claude", ".claude/.credentials.json")
        return store

    def test_real_claude_wrapper_invalid_oauth_preserves_entire_snapshot(self):
        store = self.claude_oauth_store()
        original = document_bytes(CLAUDE_OAUTH)
        original_api = b'{"primaryApiKey":"fixture-original"}'
        for description, invalid in INVALID_CLAUDE_OAUTH.items():
            for child_exit in (0, 42):
                with self.subTest(content=description, child_exit=child_exit):
                    (store / ".claude.json").write_bytes(original_api)
                    (store / ".claude/.credentials.json").write_bytes(original)
                    result = self.auth_command("claude", original, invalid, child_exit,
                                               other=(".claude.json", b'{"primaryApiKey":"fixture-updated"}'))
                    saved_oauth = store / ".claude/.credentials.json"
                    self.assertEqual((result.returncode, (store / ".claude.json").read_bytes(),
                                      saved_oauth.read_bytes() if saved_oauth.exists() else None,
                                      list(self.tmp.glob("agentcage-login-*"))),
                                     (1 if child_exit == 0 else 42, original_api, original, []), result.stderr)
                    self.assertIn(b"failed to save login", result.stderr)
                    self.assertNotIn(b"fixture-original", result.stderr)

    def test_real_claude_wrapper_oauth_refresh_nullable_fields_dead_token_and_deletion(self):
        store = self.claude_oauth_store()
        original = document_bytes(CLAUDE_OAUTH)
        original_api = b'{"primaryApiKey":"fixture-original"}'
        variants = {
            "refresh": {"accessToken": "fixture-updated-access", "refreshToken": "fixture-updated-refresh",
                        "expiresAt": 2000, "refreshTokenExpiresAt": 3000,
                        "scopes": ["user:inference", "user:profile"],
                        "clientId": "fixture-client", "subscriptionType": "fixture-subscription",
                        "rateLimitTier": "fixture-tier"},
            "minimal": {"accessToken": "fixture-updated-minimal"},
            "nullable": {"accessToken": "fixture-updated-nullable", "refreshToken": None,
                         "expiresAt": None, "refreshTokenExpiresAt": None, "scopes": None,
                         "clientId": None, "subscriptionType": None, "rateLimitTier": None},
            "native dead token": {"accessToken": "", "refreshToken": "", "expiresAt": 0},
            "deletion": None,
        }
        for description, tokens in variants.items():
            for child_exit in (0, 42):
                with self.subTest(content=description, child_exit=child_exit):
                    (store / ".claude.json").write_bytes(original_api)
                    stored = store / ".claude/.credentials.json"
                    stored.parent.mkdir(parents=True, exist_ok=True, mode=0o700)
                    stored.write_bytes(original)
                    encoded = document_bytes({"claudeAiOauth": tokens}) if tokens is not None else None
                    result = self.auth_command("claude", original, encoded, child_exit)
                    self.assertEqual(result.returncode, child_exit, result.stderr)
                    self.assertEqual(stored.exists(), description not in ("native dead token", "deletion"))
                    if stored.exists():
                        self.assertEqual(stored.read_bytes(), encoded)
                        self.assertEqual(stored.stat().st_mode & 0o777, 0o600)
                    self.assertEqual((store / ".claude.json").read_bytes(), original_api)
                    self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_claude_wrapper_invalid_types_are_not_logout(self):
        original = b'{"primaryApiKey":"fixture-original"}'
        stored = self.home / "data/agentcage/logins/claude/.claude.json"
        stored.parent.mkdir(parents=True)
        self.write_claude_fixture()
        for invalid in ('{"primaryApiKey":false}', '{"primaryApiKey":42}', '{"primaryApiKey":["fixture"]}',
                        '{"primaryApiKey":null}', 'false', '42', '["fixture"]'):
            with self.subTest(content=invalid):
                stored.write_bytes(original)
                result = subprocess.run([str(AC), "claude"], cwd=self.project,
                                        env=self.env | {"FIXTURE_CLAUDE_JSON": invalid},
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b"invalid credential structure", result.stderr)
                self.assertEqual(stored.read_bytes(), original)
                self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def write_claude_fixture(self):
        fixture = self.tools / "claude"
        fixture.write_text("""#!/usr/bin/env python3
import os,pathlib
path=pathlib.Path(os.environ['HOME'])/'.claude.json'
assert path.read_bytes()==b'{"primaryApiKey":"fixture-original"}'
path.write_text(os.environ['FIXTURE_CLAUDE_JSON'])
""")
        fixture.chmod(0o755)

    def test_real_claude_wrapper_auth_free_document_is_logout(self):
        stored = self.home / "data/agentcage/logins/claude/.claude.json"
        stored.parent.mkdir(parents=True)
        stored.write_bytes(b'{"primaryApiKey":"fixture-original"}')
        self.write_claude_fixture()
        result = subprocess.run([str(AC), "claude"], cwd=self.project,
                                env=self.env | {"FIXTURE_CLAUDE_JSON": '{"fixtureSetting":true}'},
                                stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse(stored.exists())
        self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def write_opencode_fixture(self):
        fixture = self.tools / "opencode"
        fixture.write_text("""#!/usr/bin/env python3
import os,pathlib,sqlite3,sys
data=pathlib.Path(os.environ['HOME'])/'.local/share/opencode'
if sys.argv[1:]==['db','SELECT 1']:
    data.mkdir(parents=True,exist_ok=True)
    with sqlite3.connect(data/'opencode.db') as db:
        db.executescript(%r)
else:
    if 'FIXTURE_OPENCODE_AUTH' in os.environ:
        (data/'auth.json').write_text(os.environ['FIXTURE_OPENCODE_AUTH'])
    else:
        (data/'account.json').write_text('{"accounts":[{"id":"fixture-corrupted"}]}')
        (data/'auth.json').unlink()
        with sqlite3.connect(data/'opencode.db') as db:
            db.execute('DROP TABLE account')
    sys.exit(int(os.environ.get('FIXTURE_OPENCODE_EXIT','0')))
""" % SCHEMA)
        fixture.chmod(0o755)

    def write_account_roundtrip_fixture(self):
        fixture = self.tools / "opencode"
        fixture.write_text("""#!/usr/bin/env python3
import json,os,pathlib,sqlite3,sys
data=pathlib.Path(os.environ['HOME'])/'.local/share/opencode'
if sys.argv[1:]==['db','SELECT 1']:
    data.mkdir(parents=True,exist_ok=True)
    with sqlite3.connect(data/'opencode.db') as db:
        db.executescript(%r)
else:
    with sqlite3.connect(data/'opencode.db') as db:
        fields=('id','email','url','access_token','refresh_token','token_expiry')
        rows=[dict(zip(fields,row)) for row in db.execute('SELECT '+','.join(fields)+' FROM account ORDER BY id')]
        state=db.execute('SELECT active_account_id,active_org_id FROM account_state WHERE id=1').fetchone()
        observed={'accounts':rows,'active_account_id':state[0] if state else None,
                  'active_org_id':state[1] if state else None}
        unrelated=db.execute('SELECT * FROM unrelated_session').fetchall()
        action=os.environ.get('FIXTURE_ACCOUNT_ACTION','unchanged')
        if action=='refresh':
            db.execute('UPDATE account SET access_token=?,refresh_token=?',
                       ('fixture-updated-account-access','fixture-updated-account-refresh'))
        elif action=='logout':
            db.execute('DELETE FROM account')
            db.execute('UPDATE account_state SET active_account_id=NULL,active_org_id=NULL')
        else:
            assert action=='unchanged', action
    print(json.dumps({'imported':observed,'unrelated':unrelated}))
    sys.exit(int(os.environ.get('FIXTURE_ACCOUNT_EXIT','0')))
""" % SCHEMA)
        fixture.chmod(0o755)

    def account_command(self, action="unchanged", child_exit=0):
        # Each import runs in a fresh owned HOME; the actual binary supplies
        # both the Bash wrapper and its auth-only SQLite bridge.
        shutil.rmtree(self.container)
        self.container.mkdir(mode=0o700)
        return subprocess.run([str(AC), "opencode"], cwd=self.project,
                              env=self.env | {"FIXTURE_ACCOUNT_ACTION": action,
                                              "FIXTURE_ACCOUNT_EXIT": str(child_exit)},
                              stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)

    def embedded_account_bridge(self):
        calls = [json.loads(path.read_text()) for path in (self.base / "podman/calls").iterdir()]
        runs = [call for call in calls if call[0] == "run"]
        self.assertEqual(len(runs), 1)
        command = runs[0][runs[0].index("agentcage/agent-tools:local") + 1:]
        self.assertEqual(command[:2], ["bash", "-c"])
        bridge = command[7 + int(command[6])]
        self.assertIn("def read_document():", bridge)
        return bridge

    def test_real_opencode_account_import_refresh_logout_and_host_listing_agree(self):
        stored = self.home / "data/agentcage/logins/opencode/.local/share/opencode"
        stored.mkdir(parents=True)
        account = stored / "account.json"
        auth = stored / "auth.json"
        original_auth = b'{"fixture":{"type":"api","key":"fixture-original"}}'
        self.write_account_roundtrip_fixture()
        for description, document in VALID_OPENCODE_ACCOUNTS.items():
            for child_exit in (0, 42):
                with self.subTest(content=description, child_exit=child_exit):
                    stored.mkdir(parents=True, exist_ok=True, mode=0o700)
                    content = (json.dumps(document, ensure_ascii=True, indent=3) + "\n").encode()
                    account.write_bytes(content)
                    auth.write_bytes(original_auth)
                    listed = self.command(["login", "list"])
                    self.assertEqual(listed.returncode, 0, listed.stderr)
                    self.assertIn(b"\n  opencode\n", listed.stdout)
                    self.assertEqual(account.read_bytes(), content)
                    result = self.account_command(child_exit=child_exit)
                    self.assertEqual(result.returncode, child_exit, result.stderr)
                    expected = document | {"accounts": sorted(document["accounts"], key=lambda row: row["id"])}
                    observed = json.loads(result.stdout)
                    self.assertEqual(observed["imported"], expected)
                    self.assertEqual(observed["unrelated"], [["fixture-session", "unrelated disposable history fixture"]])
                    self.assertEqual(auth.read_bytes(), original_auth)
                    if document["accounts"]:
                        self.assertEqual(json.loads(account.read_bytes()), expected)
                        self.assertEqual(account.stat().st_mode & 0o777, 0o600)
                        self.assertEqual(account.stat().st_nlink, 1)
                    else:
                        # An already auth-free host document is outside the
                        # material snapshot. An unchanged session leaves it
                        # intact; a logout from nonempty accounts below must
                        # still remove the saved account credentials.
                        self.assertEqual(account.read_bytes(), content)
                    self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])
        for action in ("refresh", "logout"):
            for child_exit in (0, 42):
                with self.subTest(action=action, child_exit=child_exit):
                    account.write_bytes(document_bytes(DOCUMENT))
                    auth.write_bytes(original_auth)
                    result = self.account_command(action, child_exit)
                    self.assertEqual(result.returncode, child_exit, result.stderr)
                    self.assertEqual(json.loads(result.stdout)["imported"], DOCUMENT)
                    self.assertEqual(auth.read_bytes(), original_auth)
                    if action == "refresh":
                        updated = DOCUMENT | {"accounts": [ROW | {
                            "access_token": "fixture-updated-account-access",
                            "refresh_token": "fixture-updated-account-refresh"}]}
                        self.assertEqual(json.loads(account.read_bytes()), updated)
                    else:
                        self.assertFalse(account.exists())
                    self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_invalid_opencode_account_host_and_embedded_bridge_both_preserve_state(self):
        stored = self.home / "data/agentcage/logins/opencode/.local/share/opencode"
        stored.mkdir(parents=True)
        account = stored / "account.json"
        auth = stored / "auth.json"
        original_auth = b'{"fixture":{"type":"api","key":"fixture-original"}}'
        account.write_bytes(document_bytes(DOCUMENT))
        auth.write_bytes(original_auth)
        self.write_account_roundtrip_fixture()
        seeded = self.account_command()
        self.assertEqual(seeded.returncode, 0, seeded.stderr)
        bridge = self.embedded_account_bridge()
        bridge_home = self.base / "bridge-home"
        bridge_data = bridge_home / ".local/share/opencode"
        bridge_data.mkdir(parents=True, mode=0o700)
        bridge_account = bridge_data / "account.json"
        database = bridge_data / "opencode.db"
        with sqlite3.connect(database) as db:
            db.executescript(SCHEMA)
        database_before = database.read_bytes()
        for description, content in INVALID_OPENCODE_ACCOUNTS.items():
            with self.subTest(content=description):
                account.write_bytes(content)
                auth.write_bytes(original_auth)
                bridge_account.write_bytes(content)
                bridge_account.chmod(0o600)
                imported = subprocess.run(["/usr/bin/python3", "-B", "-c", bridge, "import"],
                                          env=self.env | {"HOME": str(bridge_home)},
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
                self.assertEqual(imported.returncode, 1, imported.stderr)
                self.assertEqual(database.read_bytes(), database_before)
                self.assertEqual(bridge_account.read_bytes(), content)
                with sqlite3.connect(database) as db:
                    self.assertEqual(db.execute("SELECT * FROM account").fetchall(), [])
                    self.assertEqual(db.execute("SELECT * FROM account_state").fetchall(), [])
                    self.assertEqual(db.execute("SELECT * FROM unrelated_session").fetchall(),
                                     [("fixture-session", "unrelated disposable history fixture")])
                listed = self.command(["login", "list"])
                self.assertEqual(listed.returncode, 1, listed.stderr)
                self.assertNotIn(b"\n  opencode\n", listed.stdout)
                calls_before = set((self.base / "podman/calls").iterdir())
                result = self.account_command()
                self.assertEqual(result.returncode, 1, result.stderr)
                calls = [json.loads(path.read_text()) for path in
                         set((self.base / "podman/calls").iterdir()) - calls_before]
                self.assertFalse(any(call[0] == "run" for call in calls),
                                 "the host accepted a document rejected by its own SQLite bridge")
                self.assertEqual(account.read_bytes(), content)
                self.assertEqual(auth.read_bytes(), original_auth)
                self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])
                for diagnostic in (imported.stderr, listed.stderr, result.stderr):
                    self.assertNotIn(b"fixture-access", diagnostic)
                    self.assertNotIn(b"fixture-refresh", diagnostic)

    def test_real_opencode_wrapper_export_failure_preserves_snapshot(self):
        stored = self.home / "data/agentcage/logins/opencode/.local/share/opencode"
        stored.mkdir(parents=True)
        original = json.dumps(DOCUMENT).encode()
        original_auth = b'{"fixture":{"type":"api","key":"fixture-original"}}'
        self.write_opencode_fixture()
        for child_exit in (0, 42):
            with self.subTest(child_exit=child_exit):
                shutil.rmtree(self.container)
                self.container.mkdir(mode=0o700)
                (stored / "account.json").write_bytes(original)
                (stored / "auth.json").write_bytes(original_auth)
                result = subprocess.run([str(AC), "opencode"], cwd=self.project,
                                        env=self.env | {"FIXTURE_OPENCODE_EXIT": str(child_exit)},
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
                self.assertEqual(result.returncode, 1 if child_exit == 0 else 42, result.stderr)
                self.assertIn(b"account sync failed", result.stderr)
                self.assertEqual((stored / "account.json").read_bytes(), original)
                self.assertEqual((stored / "auth.json").read_bytes(), original_auth)
                self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_opencode_wrapper_invalid_provider_is_not_logout(self):
        stored = self.home / "data/agentcage/logins/opencode/.local/share/opencode"
        stored.mkdir(parents=True)
        original = b'{"fixture":{"type":"api","key":"fixture-original"}}'
        self.write_opencode_fixture()
        for invalid in ('{"fixture":{}}', '{"fixture":{"type":"api"}}',
                        '{"fixture":{"type":"api","key":false}}',
                        '{"fixture":{"type":"api","key":"fixture","metadata":[]}}',
                        '{"fixture":{"type":"oauth","access":"fixture","refresh":false,"expires":42}}',
                        '{"fixture":{"type":"oauth","access":"fixture","refresh":"fixture","expires":-1}}',
                        '{"fixture":{"type":"oauth","access":"fixture","refresh":"fixture","expires":0.5}}',
                        '{"fixture":{"type":"oauth","access":"fixture","refresh":"fixture","expires":1e999}}',
                        '{"fixture":{"type":"unknown","key":"fixture"}}'):
            with self.subTest(content=invalid):
                shutil.rmtree(self.container)
                self.container.mkdir(mode=0o700)
                (stored / "auth.json").write_bytes(original)
                result = subprocess.run([str(AC), "opencode"], cwd=self.project,
                                        env=self.env | {"FIXTURE_OPENCODE_AUTH": invalid},
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(b"invalid credential structure", result.stderr)
                self.assertEqual((stored / "auth.json").read_bytes(), original)
                self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])

    def test_real_opencode_wrapper_provider_variants_and_logout(self):
        stored = self.home / "data/agentcage/logins/opencode/.local/share/opencode"
        stored.mkdir(parents=True)
        self.write_opencode_fixture()
        for content in ('{"fixture":{"type":"api","key":"fixture-updated","metadata":{"fixture":"value"}}}',
                        '{"fixture":{"type":"oauth","access":"fixture-access","refresh":"fixture-refresh","expires":1e3,"accountId":"fixture-account"}}',
                        '{"fixture":{"type":"wellknown","key":"fixture-key","token":"fixture-token"}}', '{}'):
            with self.subTest(content=content):
                shutil.rmtree(self.container)
                self.container.mkdir(mode=0o700)
                (stored / "auth.json").write_bytes(b'{"fixture":{"type":"api","key":"fixture-original"}}')
                result = subprocess.run([str(AC), "opencode"], cwd=self.project,
                                        env=self.env | {"FIXTURE_OPENCODE_AUTH": content},
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=20)
                self.assertEqual(result.returncode, 0, result.stderr)
                if content == '{}':
                    self.assertFalse((stored / "auth.json").exists())
                else:
                    self.assertEqual((stored / "auth.json").read_bytes(), content.encode())
                    self.assertEqual((stored / "auth.json").stat().st_mode & 0o777, 0o600)
                self.assertFalse((stored / "account.json").exists())
                self.assertEqual(list(self.tmp.glob("agentcage-login-*")), [])


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    args, remaining = parser.parse_known_args()
    with tempfile.TemporaryDirectory(prefix="ac-wrapper-binary-") as binary_dir:
        AC = Path(binary_dir) / "ac"
        shutil.copy2(args.binary.resolve(), AC)
        unittest.main(argv=[__file__, *remaining])

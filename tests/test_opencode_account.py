#!/usr/bin/env python3
"""Auth-only SQLite bridge effects, using exclusively synthetic account data."""
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import tempfile
import unittest


BRIDGE = Path(__file__).resolve().parents[1] / "src/opencode_account.py"
ROW = {"id": "fixture-account", "email": "fixture@example.invalid", "url": "https://example.invalid",
       "access_token": "fixture-access", "refresh_token": "fixture-refresh", "token_expiry": 1700000000000}
DOCUMENT = {"accounts": [ROW], "active_account_id": "fixture-account", "active_org_id": "fixture-org"}
SCHEMA = """
CREATE TABLE account (id TEXT PRIMARY KEY,email TEXT NOT NULL,url TEXT NOT NULL,
    access_token TEXT NOT NULL,refresh_token TEXT NOT NULL,token_expiry INTEGER,time_created INTEGER,time_updated INTEGER);
CREATE TABLE account_state (id INTEGER PRIMARY KEY,active_account_id TEXT,active_org_id TEXT);
CREATE TABLE unrelated_session (id TEXT PRIMARY KEY,content TEXT);
INSERT INTO unrelated_session VALUES ('fixture-session','unrelated disposable history fixture');
"""


class AccountBridgeTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ac-account-")
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.data = self.home / ".local/share/opencode"
        self.data.mkdir(parents=True, mode=0o700)
        self.dbfile = self.data / "opencode.db"
        self.account = self.data / "account.json"
        with sqlite3.connect(self.dbfile) as db:
            db.executescript(SCHEMA)
        self.env = {"PATH": "/usr/bin:/bin", "HOME": str(self.home), "LANG": "C.UTF-8",
                    "XDG_DATA_HOME": str(self.home / ".local/share"), "XDG_CONFIG_HOME": str(self.home / ".config"),
                    "XDG_STATE_HOME": str(self.home / ".state"), "XDG_CACHE_HOME": str(self.home / ".cache"),
                    "TMPDIR": str(self.home)}

    def invoke(self, operation):
        return subprocess.run(["python3", str(BRIDGE), operation], env=self.env, stdout=subprocess.PIPE,
                              stderr=subprocess.PIPE, timeout=15)

    def write(self, document=DOCUMENT):
        self.account.write_text(json.dumps(document))
        self.account.chmod(0o600)

    def rows(self):
        with sqlite3.connect(self.dbfile) as db:
            return db.execute("SELECT id,access_token,refresh_token FROM account ORDER BY id").fetchall()

    def test_import_refresh_export_and_logout_preserve_unrelated_tables(self):
        self.write()
        result = self.invoke("import")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.rows(), [("fixture-account", "fixture-access", "fixture-refresh")])
        with sqlite3.connect(self.dbfile) as db:
            self.assertEqual(db.execute("SELECT * FROM account_state").fetchall(), [(1, "fixture-account", "fixture-org")])
            db.execute("UPDATE account SET access_token=?,refresh_token=?", ("fixture-new-access", "fixture-new-refresh"))
        result = self.invoke("export")
        self.assertEqual(result.returncode, 0, result.stderr)
        document = json.loads(self.account.read_text())
        self.assertEqual(document["accounts"][0]["access_token"], "fixture-new-access")
        self.assertEqual(document["accounts"][0]["refresh_token"], "fixture-new-refresh")
        self.assertEqual(set(document["accounts"][0]), set(ROW))
        self.assertNotIn("unrelated disposable history", self.account.read_text())
        self.assertEqual(self.account.stat().st_mode & 0o777, 0o600)
        with sqlite3.connect(self.dbfile) as db:
            db.execute("DELETE FROM account")
            db.execute("UPDATE account_state SET active_account_id=NULL,active_org_id=NULL")
        self.assertEqual(self.invoke("export").returncode, 0)
        self.assertEqual(json.loads(self.account.read_text()), {"accounts": [], "active_account_id": None, "active_org_id": None})
        with sqlite3.connect(self.dbfile) as db:
            self.assertEqual(db.execute("SELECT * FROM unrelated_session").fetchall(), [("fixture-session", "unrelated disposable history fixture")])

    def test_whitespace_unicode_null_expiry_and_multiple_accounts(self):
        second = ROW | {"id": "fixture-second", "email": "druhy☃@example.invalid", "token_expiry": None}
        document = DOCUMENT | {"accounts": [ROW, second]}
        self.account.write_text(json.dumps(document, ensure_ascii=True, indent=3))
        self.assertEqual(self.invoke("import").returncode, 0)
        self.assertEqual(self.invoke("export").returncode, 0)
        self.assertEqual(json.loads(self.account.read_text()), document)

    def test_strict_schema_and_types_reject_before_changing_database(self):
        invalid = [DOCUMENT | {"extra": "fixture"}, DOCUMENT | {"accounts": False},
                   DOCUMENT | {"active_account_id": "missing"}, DOCUMENT | {"active_org_id": False},
                   DOCUMENT | {"accounts": [ROW, ROW]}, DOCUMENT | {"accounts": [ROW | {"extra": "fixture"}]},
                   DOCUMENT | {"accounts": [ROW | {"email": ""}]}]
        for expiry in (True, -1, 2**63, 1.5, "soon"):
            invalid.append(DOCUMENT | {"accounts": [ROW | {"token_expiry": expiry}]})
        for document in invalid:
            with self.subTest(document=document):
                self.write(document)
                before = self.account.read_bytes()
                result = self.invoke("import")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.rows(), [])
                self.assertEqual(self.account.read_bytes(), before)
                self.assertNotIn(b"fixture-access", result.stderr)
                self.assertNotIn(b"fixture-refresh", result.stderr)

    def test_duplicate_keys_invalid_json_and_input_bounds_fail_without_token_excerpts(self):
        for text in ('{"accounts":[],"accounts":[]}', '{"accounts":[', '{"accounts":NaN}',
                     '{"accounts":' + json.dumps([ROW]) + ',"active_account_id":null,"active_org_id":null,"active_org_id":"fixture"}',
                     "x" * (10 * 1024 * 1024 + 1)):
            with self.subTest(length=len(text)):
                self.account.write_text(text)
                result = self.invoke("import")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.rows(), [])
                self.assertNotIn(b"fixture-access", result.stderr)
                self.assertEqual(list(self.data.glob(".account-*")), [])

    def test_existing_accounts_and_unknown_schema_are_not_overwritten(self):
        self.write()
        self.assertEqual(self.invoke("import").returncode, 0)
        self.write(DOCUMENT | {"accounts": [ROW | {"access_token": "fixture-other"}]})
        self.assertNotEqual(self.invoke("import").returncode, 0)
        self.assertEqual(self.rows()[0][1], "fixture-access")
        before = self.account.read_bytes()
        with sqlite3.connect(self.dbfile) as db:
            db.execute("ALTER TABLE account ADD COLUMN unknown_field TEXT")
        for operation in ("import", "export"):
            with self.subTest(operation=operation):
                self.assertNotEqual(self.invoke(operation).returncode, 0)
                self.assertEqual(self.account.read_bytes(), before)

    def test_export_reads_database_without_mutation_and_rejects_invalid_db_rows(self):
        self.write()
        self.assertEqual(self.invoke("import").returncode, 0)
        self.dbfile.chmod(0o400)
        before = hashlib.sha256(self.dbfile.read_bytes()).hexdigest()
        self.assertEqual(self.invoke("export").returncode, 0)
        self.assertEqual(hashlib.sha256(self.dbfile.read_bytes()).hexdigest(), before)
        self.dbfile.chmod(0o600)
        previous = self.account.read_bytes()
        with sqlite3.connect(self.dbfile) as db:
            db.execute("UPDATE account SET token_expiry=-1")
        result = self.invoke("export")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.account.read_bytes(), previous)
        self.assertEqual(list(self.data.glob(".account-*")), [])

    def test_input_and_database_symlinks_hardlinks_and_fifo_are_rejected(self):
        outside = self.home / "outside.json"
        outside.write_text(json.dumps(DOCUMENT))
        for kind in ("symlink", "hardlink", "fifo"):
            with self.subTest(kind=kind):
                self.account.unlink(missing_ok=True)
                if kind == "symlink":
                    self.account.symlink_to(outside)
                elif kind == "hardlink":
                    os.link(outside, self.account)
                else:
                    os.mkfifo(self.account)
                result = self.invoke("import")
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.rows(), [])
        self.account.unlink()
        self.write()
        original = self.home / "original.db"
        self.dbfile.rename(original)
        self.dbfile.symlink_to(original)
        for operation in ("import", "export"):
            self.assertNotEqual(self.invoke(operation).returncode, 0)

    def test_account_count_and_encoded_output_are_bounded(self):
        self.write()
        previous = self.account.read_bytes()
        with sqlite3.connect(self.dbfile) as db:
            db.executemany("INSERT INTO account VALUES (?,?,?,?,?,?,?,?)", [
                ("fixture-" + str(i), "fixture@example.invalid", "https://example.invalid", "fixture-access", "fixture-refresh", None, 0, 0)
                for i in range(10001)])
        result = self.invoke("export")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.account.read_bytes(), previous)
        with sqlite3.connect(self.dbfile) as db:
            db.execute("DELETE FROM account")
            db.execute("INSERT INTO account VALUES (?,?,?,?,?,?,?,?)", (ROW["id"], ROW["email"], ROW["url"],
                       "x" * (10 * 1024 * 1024), ROW["refresh_token"], None, 0, 0))
        result = self.invoke("export")
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(self.account.read_bytes(), previous)
        self.assertEqual(list(self.data.glob(".account-*")), [])


if __name__ == "__main__":
    unittest.main()

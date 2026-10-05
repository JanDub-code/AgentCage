"""Embedded auth-only bridge for the pinned OpenCode 1.18.34 container.

OpenCode's account and account_state tables are defined in
packages/core/src/account/sql.ts in the upstream v1.18.34 tag. Its database,
session tables, logs and migration journal stay in the disposable home.
Only the explicit account fields below leave that home as account.json.
"""
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys
import tempfile
import time


MAX_BYTES = 10 * 1024 * 1024
MAX_ACCOUNTS = 10000
FIELDS = ("id", "email", "url", "access_token", "refresh_token", "token_expiry")
DATA = Path(os.environ["HOME"]) / ".local/share/opencode"
ACCOUNT = DATA / "account.json"
DATABASE = DATA / "opencode.db"


def regular(path):
    info = path.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise ValueError("account input must be a regular, unlinked file")


def account(row):
    if not isinstance(row, dict) or set(row) != set(FIELDS):
        raise ValueError("unsupported account fields")
    for name in FIELDS[:-1]:
        if not isinstance(row[name], str) or not row[name]:
            raise ValueError("invalid account field: " + name)
    expiry = row["token_expiry"]
    if expiry is not None and (type(expiry) is not int or not 0 <= expiry < 2**63):
        raise ValueError("invalid account token expiry")
    return row


def document(value):
    if not isinstance(value, dict) or set(value) != {"accounts", "active_account_id", "active_org_id"}:
        raise ValueError("unsupported account document")
    rows = value["accounts"]
    if not isinstance(rows, list) or len(rows) > MAX_ACCOUNTS:
        raise ValueError("invalid account list")
    identifiers = set()
    for row in rows:
        account(row)
        if row["id"] in identifiers:
            raise ValueError("duplicate account identifier")
        identifiers.add(row["id"])
    active = value["active_account_id"]
    org = value["active_org_id"]
    if active is not None and (not isinstance(active, str) or active not in identifiers):
        raise ValueError("invalid active account")
    if org is not None and (not isinstance(org, str) or not org or active is None):
        raise ValueError("invalid active organization")
    return value


def read_document():
    try:
        descriptor = os.open(ACCOUNT, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    except FileNotFoundError:
        return None
    with os.fdopen(descriptor, "rb") as source:
        info = os.fstat(source.fileno())
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise ValueError("account input must be a regular, unlinked file")
        data = source.read(MAX_BYTES + 1)
    if len(data) > MAX_BYTES:
        raise ValueError("account input exceeds 10 MiB")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError("duplicate account JSON key")
            result[key] = value
        return result
    return document(json.loads(data, object_pairs_hook=unique))


def connect(mode):
    # The caller initialized the pinned agent's own schema. Never create a
    # replacement schema or silently export an unknown database version.
    regular(DATABASE)
    db = sqlite3.connect(DATABASE.as_uri() + "?mode=" + mode, uri=True, timeout=5)
    db.execute("PRAGMA foreign_keys=ON")
    expected = {"account": set(FIELDS) | {"time_created", "time_updated"},
                "account_state": {"id", "active_account_id", "active_org_id"}}
    for table, columns in expected.items():
        actual = {row[1] for row in db.execute("PRAGMA table_info(" + table + ")")}
        if actual != columns:
            db.close()
            raise ValueError("unsupported OpenCode account schema")
    return db


def restore():
    value = read_document()
    if value is None:
        return
    with connect("rw") as db:
        if db.execute("SELECT 1 FROM account LIMIT 1").fetchone() is not None:
            raise ValueError("refusing to replace existing session accounts")
        now = int(time.time() * 1000)
        for row in value["accounts"]:
            db.execute("INSERT INTO account (" + ",".join(FIELDS) + ",time_created,time_updated) VALUES (?,?,?,?,?,?,?,?)",
                       [row[name] for name in FIELDS] + [now, now])
        db.execute("INSERT OR REPLACE INTO account_state (id,active_account_id,active_org_id) VALUES (1,?,?)",
                   (value["active_account_id"], value["active_org_id"]))


def save():
    with connect("ro") as db:
        rows = db.execute("SELECT " + ",".join(FIELDS) + " FROM account ORDER BY id").fetchmany(MAX_ACCOUNTS + 1)
        state = db.execute("SELECT active_account_id,active_org_id FROM account_state WHERE id=1").fetchone()
        value = document({"accounts": [dict(zip(FIELDS, row)) for row in rows],
                          "active_account_id": state[0] if state else None,
                          "active_org_id": state[1] if state else None})
    encoded = (json.dumps(value, ensure_ascii=True, sort_keys=True, separators=(",", ":")) + "\n").encode()
    if len(encoded) > MAX_BYTES:
        raise ValueError("account output exceeds 10 MiB")
    descriptor, temporary = tempfile.mkstemp(prefix=".account-", dir=DATA)
    try:
        with os.fdopen(descriptor, "wb") as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary, ACCOUNT)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)


try:
    if sys.argv[1:] == ["import"]:
        restore()
    elif sys.argv[1:] == ["export"]:
        save()
    else:
        raise ValueError("invalid bridge operation")
except (OSError, ValueError, sqlite3.Error, RecursionError):
    # Never include SQL rows, credential values, or JSON parser excerpts.
    print("agentcage: OpenCode account sync failed; check account format/schema and private file permissions", file=sys.stderr)
    sys.exit(1)

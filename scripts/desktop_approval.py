"""E9 (ADR-12, AC-7.1.8): checking a Vajra Desktop approval.

The app signs an approval payload with a Secure Enclave P-256 key that signs only after macOS owner
authentication. The user enrolls the public key with `vajra_kb.py desktop-enroll` in their own terminal.
An approval counts only if its signature verifies against that key. Where the file lives is not trusted.

File: {"payload": {...}, "signature": "<base64 DER ECDSA over the canonical payload JSON>"}.
Canonical payload JSON: sort_keys, separators (",", ":"), ensure_ascii false, UTF-8.

This module is pure: it checks one file against one public key. The used-approval set, the enrolled key
and the store lock live in vajra_kb.py (the HMAC-chained ledger).
"""
import base64
import binascii
import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import tempfile
import time

SCHEMA = "vajra.desktop-approval/v1"
MAX_AGE = 300
MAX_FILE = 65536
ACTIONS = ("approve", "reject", "forget", "retire", "supersede")
WIDEN = ("always", "global")
ID_RE = re.compile(r"^[0-9a-f]{32}$")
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
OPENSSL = "/usr/bin/openssl"


class Refused(Exception):
    pass


def canon_bytes(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode("utf-8")


def _openssl(args):
    try:
        r = subprocess.run([OPENSSL] + args, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                           stderr=subprocess.DEVNULL, timeout=15, env={"PATH": "/usr/bin:/bin", "LC_ALL": "C"})
    except (OSError, subprocess.SubprocessError):
        raise Refused("openssl is not available")
    return r.returncode, r.stdout


def read_small(path, limit):
    """The bytes of a regular file, read once, no symlink followed."""
    try:
        fd = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(fd, "rb") as f:
            if not stat.S_ISREG(os.fstat(f.fileno()).st_mode):
                raise Refused("%s is not a regular file" % path)
            raw = f.read(limit + 1)
    except OSError as e:
        raise Refused("cannot read %s: %s" % (path, e.strerror or e))
    if len(raw) > limit:
        raise Refused("%s is too large" % path)
    return raw


def public_key(path):
    """(pem, fingerprint) of a P-256 public key file; the fingerprint is sha256 of its DER."""
    raw = read_small(path, 4096)
    d = tempfile.mkdtemp(prefix="vajra-e9-")
    try:
        p = os.path.join(d, "k.pem")
        with open(p, "wb") as f:
            f.write(raw)
        rc, pem = _openssl(["pkey", "-pubin", "-in", p, "-outform", "PEM"])
        rc2, der = _openssl(["pkey", "-pubin", "-in", p, "-outform", "DER"])
        rc3, text = _openssl(["ec", "-pubin", "-in", p, "-noout", "-text"])
    finally:
        shutil.rmtree(d, ignore_errors=True)
    if rc or rc2 or not pem or not der:
        raise Refused("not a PEM public key")
    if rc3 or b"prime256v1" not in text:
        raise Refused("not a P-256 (prime256v1) key")
    return pem.decode("ascii"), hashlib.sha256(der).hexdigest()


def _signature_ok(pem, message, sig):
    d = tempfile.mkdtemp(prefix="vajra-e9-")
    try:
        paths = []
        for name, data in (("k.pem", pem.encode("ascii")), ("m", message), ("s", sig)):
            p = os.path.join(d, name)
            with open(p, "wb") as f:
                f.write(data)
            paths.append(p)
        rc, out = _openssl(["dgst", "-sha256", "-verify", paths[0], "-signature", paths[2], paths[1]])
    finally:
        shutil.rmtree(d, ignore_errors=True)
    return rc == 0 and b"Verified OK" in out


def verify(path, pem, action, store, record, now=None):
    """The payload of the approval at `path`, or Refused. Checks the signature against `pem`, the schema,
    the 5 minute age and that it is for exactly this action, store and record."""
    raw = read_small(path, MAX_FILE)
    try:
        doc = json.loads(raw.decode("utf-8"))
        payload, sig = doc["payload"], base64.b64decode(doc["signature"], validate=True)
    except (ValueError, KeyError, TypeError, binascii.Error):
        raise Refused("%s is not an approval file" % os.path.basename(path))
    if not isinstance(payload, dict) or set(doc) != {"payload", "signature"} or not sig:
        raise Refused("%s is not an approval file" % os.path.basename(path))
    if not _signature_ok(pem, canon_bytes(payload), sig):
        raise Refused("the approval's signature does not verify against the enrolled desktop key")
    p, t = payload, (time.time() if now is None else now)
    created = p.get("created")
    if p.get("schema") != SCHEMA or not isinstance(p.get("id"), str) or not ID_RE.match(p["id"]) \
            or p.get("action") not in ACTIONS or not isinstance(p.get("sha256"), str) \
            or not SHA_RE.match(p["sha256"]):
        raise Refused("the approval is malformed")
    if not isinstance(created, (int, float)) or isinstance(created, bool) or not (t - MAX_AGE < created <= t + 5):
        raise Refused("the approval is expired (5 minutes) or from the future")
    if (p["action"], p.get("store"), p.get("record")) != (action, store, record):
        raise Refused("the approval is for %s %s in %s, not %s %s in %s"
                      % (p["action"], p.get("record"), p.get("store"), action, record, store))
    return p


def verify_all(paths, pem, action, store, record, now=None):
    """(main, {widen: confirmation}). Confirmations are separate signed approvals for `always` and
    global scope, each naming the main approval."""
    if not paths:
        raise Refused("no approval given")
    main = verify(paths[0], pem, action, store, record, now)
    if "confirms" in main or "widen" in main:
        raise Refused("the first approval must be the action, not a confirmation")
    conf, seen = {}, {main["id"]}
    for path in paths[1:]:
        c = verify(path, pem, action, store, record, now)
        w = c.get("widen")
        if c["id"] in seen or c.get("confirms") != main["id"] or w not in WIDEN or w in conf \
                or c["sha256"] != main["sha256"]:
            raise Refused("confirmation %s does not confirm approval %s" % (c["id"], main["id"]))
        seen.add(c["id"])
        conf[w] = c
    return main, conf


def audit(action, a, sha256, store, applies=None):
    """One line in ~/.claude/vajra/audit.log: ids and hashes only, never record text."""
    home = os.environ.get("HOME") or os.path.expanduser("~")
    path = os.path.join(home, ".claude", "vajra", "audit.log")
    line = "%s desktop-approval action=%s record=%s sha256=%s store=%s%s approval=%s\n" % (
        time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), action, a.get("record"), sha256, store,
        (" applies=%s" % applies) if applies else "", a["id"])
    # Runs after the change is committed: a failure here must not look like a failed approval.
    try:
        os.makedirs(os.path.dirname(path), 0o700, exist_ok=True)
        fd = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_CREAT, 0o600)
        with os.fdopen(fd, "a") as f:
            f.write(line)
    except OSError as e:
        import sys
        sys.stderr.write("vajra_kb: approved, but audit.log could not be written: %s\n" % (e.strerror or e))

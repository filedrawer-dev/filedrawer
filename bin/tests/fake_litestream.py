"""A stand-in for Litestream 0.5's `replicate -once` and `restore`, for
ops/b2-probe's tests, against bin/tests/fake_b2.py's S3.

It reads the config the probe writes (JSON, which Litestream reads as YAML) and
uploads the whole database file as each LTX file, as Litestream's S3 client would:
a single PUT, with Content-MD5 when put-object-content-md5 is on, below the part
size, and a multipart upload, with no checksum, at or above it. Its record of the
files it wrote lives in the database's -litestream folder, as Litestream's does.
It logs and exits 1 on any refusal, and exits 9 when the environment holds an AWS_
setting beyond the key, which a probe that passed its own environment on would.
"""

import base64
import hashlib
import json
import os
import re
import sys
import urllib.error
import urllib.request
from pathlib import Path


def fail(message: str) -> None:
    print(f'level=ERROR msg="sync error" error="{message}"', flush=True)
    sys.exit(1)


def size(text: str | None) -> int:
    if not text:
        return 5 * 1024 * 1024
    number, unit = re.fullmatch(r"(\d+)(MiB|B)", text).groups()
    return int(number) * (1024 * 1024 if unit == "MiB" else 1)


class Replica:
    def __init__(self, config: dict) -> None:
        db = config["dbs"][0]
        self.db = Path(db["path"])
        self.replica = db["replica"]
        self.meta = self.db.with_name(self.db.name + "-litestream")
        self.meta.mkdir(exist_ok=True)
        self.record = self.meta / "fake.json"

    def files(self) -> list[str]:
        return json.loads(self.record.read_text()) if self.record.exists() else []

    def remember(self, keys: list[str]) -> None:
        self.record.write_text(json.dumps(keys))

    def request(
        self, method: str, key: str, query: str = "", body: bytes = b"", headers=None
    ):
        url = f"{self.replica['endpoint']}/{self.replica['bucket']}/{key}" + (
            f"?{query}" if query else ""
        )
        auth = f"AWS4-HMAC-SHA256 Credential={os.environ['AWS_ACCESS_KEY_ID']}/x, Signature=0"
        request = urllib.request.Request(
            url,
            data=body or None,
            method=method,
            headers={"Authorization": auth, **(headers or {})},
        )
        try:
            with urllib.request.urlopen(request) as reply:
                return reply.read()
        except urllib.error.HTTPError as error:
            fail(f"{method} {key}: {error.code} {error.read().decode()}")

    def put(self, key: str, data: bytes) -> None:
        if len(data) >= size(self.replica.get("part-size")):
            self.request("POST", key, "uploads=")
            self.request("PUT", key, body=data, headers={"X-Fake-Multipart": "1"})
            return
        headers = {}
        if self.replica.get("put-object-content-md5"):
            headers["Content-MD5"] = base64.b64encode(
                hashlib.md5(data).digest()
            ).decode()
        self.request("PUT", key, body=data, headers=headers)

    def write(self, level: int) -> str:
        keys = self.files()
        txid = len(keys) + 1
        key = f"{self.replica['path']}/{level:04x}/{txid:016x}-{txid:016x}.ltx"
        self.put(key, self.db.read_bytes())
        self.remember([*keys, key])
        return key

    def enforce_retention(self) -> None:
        snapshots = [k for k in self.files() if "/0009/" in k]
        old = snapshots[:-1]
        if not old:
            return
        objects = "".join(f"<Object><Key>{k}</Key></Object>" for k in old)
        body = f"<Delete>{objects}</Delete>".encode()
        md5 = base64.b64encode(hashlib.md5(body).digest()).decode()
        reply = self.request("POST", "", "delete=", body, {"Content-MD5": md5})
        if b"<Error>" in reply:
            fail(f"delete: {reply.decode()}")
        self.remember([k for k in self.files() if k not in old])


def main(argv: list[str]) -> None:
    extra = sorted(k for k in os.environ if k.startswith("AWS_"))
    if extra != ["AWS_ACCESS_KEY_ID", "AWS_SECRET_ACCESS_KEY"]:
        print(f"unexpected environment: {extra}", flush=True)
        sys.exit(9)
    command, flag, config_path, *rest = argv
    assert flag == "-config", argv
    replica = Replica(json.loads(Path(config_path).read_text()))
    if command == "replicate":
        replica.write(0)
        if "-force-snapshot" in rest:
            replica.write(9)
        if "-enforce-retention" in rest:
            replica.enforce_retention()
    elif command == "reset":
        # Litestream's reset removes the local state, so the next sync starts afresh.
        replica.record.unlink(missing_ok=True)
    elif command == "restore":
        out = Path(rest[rest.index("-o") + 1])
        newest = [k for k in replica.files() if "/0009/" in k][-1]
        out.write_bytes(replica.request("GET", newest))
    print("one-shot replication complete", flush=True)


if __name__ == "__main__":
    main(sys.argv[1:])

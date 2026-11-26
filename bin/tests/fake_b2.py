"""A stand-in for Backblaze B2 for ops/b2-probe's tests: the native API calls the
probe makes (v4) and the S3 requests it and Litestream make, on one local port.

It keeps buckets and their file versions in memory, on a clock of its own that a
test moves with `advance`. It follows what B2's docs say and #156 asks the probe
to confirm: a key may act only on its own buckets and with its capabilities; a
delete by name or a hide adds a hide marker and keeps the version; deleting a
version needs deleteFiles and is refused while its compliance lock holds; a
bucket must be empty to be deleted. Two switches change B2's answers for a test:
`refuse_multipart_in_lock` (B2 wants a checksum on each part in a locked bucket)
and `restricted_can_delete` (a key gets deleteFiles though the probe didn't ask).
S3 signatures aren't checked: the key is read from the Authorization header's
Credential.
"""

import base64
import http.server
import itertools
import json
import re
import threading
import time
import urllib.parse

from conftest import serve_in_background

MASTER_ID = "master-key-id-0001"
MASTER_KEY = "MASTER-secret-never-printed-0001"
ALL_CAPABILITIES = [
    "listBuckets", "writeBuckets", "deleteBuckets", "listKeys", "writeKeys",
    "deleteKeys", "listFiles", "readFiles", "writeFiles", "deleteFiles",
    "readFileRetentions", "writeFileRetentions", "writeBucketRetentions",
]  # fmt: skip
DAY = 86400
LOCK_MESSAGE = (
    "Content-MD5 OR x-amz-checksum- HTTP header is required for Put Object requests "
    "with Object Lock parameters"
)


class FakeB2:
    def __init__(self) -> None:
        self.clock = time.time()
        self.lock = threading.Lock()
        self.ids = itertools.count(1)
        self.keys = {
            MASTER_ID: {"secret": MASTER_KEY, "caps": ALL_CAPABILITIES, "buckets": None}
        }
        self.tokens: dict[str, str] = {}
        self.buckets: dict[str, dict] = {}
        self.s3_requests: list[tuple[str, str, str]] = []
        self.refuse_multipart_in_lock = True
        self.restricted_can_delete = False
        self.lifecycle_retries_after_lock = True
        self.lifecycle_ignores_lock = False
        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), self.handler())
        self.url = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.thread = serve_in_background(self.server)

    def stop(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join()

    def now(self) -> float:
        return self.clock

    def advance(self, days: float) -> None:
        self.clock += days * DAY

    def new_id(self, prefix: str) -> str:
        return f"{prefix}{next(self.ids):06d}"

    def bucket_named(self, name: str) -> dict | None:
        return next((b for b in self.buckets.values() if b["name"] == name), None)

    def restricted_keys(self) -> list[str]:
        return [k for k in self.keys if k != MASTER_ID]

    def run_lifecycle(self) -> None:
        """B2's daily pass: delete each version hidden a day or more ago, unless its
        lock holds; with lifecycle_retries_after_lock off, a version skipped once for
        its lock is never deleted."""
        for bucket in self.buckets.values():
            if not bucket["lifecycle"]:
                continue
            for name in {v["fileName"] for v in bucket["versions"]}:
                ordered = newest_first(bucket["versions"], name)
                for older, newer in zip(ordered[1:], ordered):
                    if older["action"] != "upload" or self.clock - newer["at"] < DAY:
                        continue
                    locked = older["until"] and older["until"] > self.clock
                    if self.lifecycle_ignores_lock:
                        locked = False
                    if locked or older.get("skipped"):
                        older["skipped"] = not self.lifecycle_retries_after_lock
                        continue
                    bucket["versions"].remove(older)

    # --- the store -----------------------------------------------------------

    def add_version(
        self, bucket: dict, name: str, action: str, data: bytes = b""
    ) -> dict:
        self.clock += 0.001  # each version strictly newer than the last
        until = (
            self.clock + bucket["retention_days"] * DAY
            if action == "upload" and bucket["retention_days"]
            else None
        )
        version = {
            "fileName": name,
            "fileId": self.new_id("4_z"),
            "action": action,
            "at": self.clock,
            "data": data,
            "until": until,
        }
        bucket["versions"].append(version)
        return version

    def listed(self, version: dict) -> dict:
        out = {
            "fileName": version["fileName"],
            "fileId": version["fileId"],
            "action": version["action"],
            "uploadTimestamp": int(version["at"] * 1000),
            "contentLength": len(version["data"]),
        }
        if version["until"]:
            out["fileRetention"] = {
                "isClientAuthorizedToRead": True,
                "value": {
                    "mode": "compliance",
                    "retainUntilTimestamp": int(version["until"] * 1000),
                },
            }
        return out

    def capable(self, key_id: str, cap: str, bucket: dict | None) -> bool:
        key = self.keys.get(key_id)
        if key is None:
            return False
        if (
            bucket is not None
            and key["buckets"] is not None
            and bucket["id"] not in key["buckets"]
        ):
            return False
        return cap in key["caps"]

    # --- the native API ------------------------------------------------------

    def native(self, name: str, key_id: str, body: dict) -> tuple[int, dict]:
        refuse = (401, {"code": "unauthorized", "message": "not allowed"})
        bucket = self.buckets.get(body.get("bucketId", ""))
        if name == "b2_create_bucket":
            if not self.capable(key_id, "writeBuckets", None):
                return refuse
            bucket_id = self.new_id("b")
            self.buckets[bucket_id] = {
                "id": bucket_id,
                "name": body["bucketName"],
                "locked": body.get("fileLockEnabled", False),
                "retention_days": 0,
                "lifecycle": body.get("lifecycleRules", []),
                "sse": body.get("defaultServerSideEncryption"),
                "versions": [],
            }
            return 200, {"bucketId": bucket_id, "bucketName": body["bucketName"]}
        if name == "b2_update_bucket":
            retention = body.get("defaultRetention")
            if retention and not bucket["locked"]:
                return 400, {
                    "code": "bad_request",
                    "message": "bucket is not Object Lock-enabled",
                }
            if retention:
                bucket["retention_days"] = retention["period"]["duration"]
            return 200, {"bucketId": bucket["id"]}
        if name == "b2_list_buckets":
            listed = [
                {"bucketId": b["id"], "bucketName": b["name"]}
                for b in self.buckets.values()
                if body.get("bucketId") in (None, b["id"])
            ]
            return 200, {"buckets": listed}
        if name == "b2_update_file_retention":
            if not self.capable(key_id, "writeFileRetentions", None):
                return refuse
            found = [
                v
                for b in self.buckets.values()
                for v in b["versions"]
                if v["fileId"] == body["fileId"]
            ]
            until = body["fileRetention"]["retainUntilTimestamp"] / 1000
            if not found or (found[0]["until"] and until < found[0]["until"]):
                return 400, {
                    "code": "bad_request",
                    "message": "a compliance lock can't shorten",
                }
            found[0]["until"] = until
            return 200, {"fileId": body["fileId"]}
        if name == "b2_create_key":
            if not self.capable(key_id, "writeKeys", None):
                return refuse
            new_id = self.new_id("005key")
            caps = list(body["capabilities"])
            if self.restricted_can_delete:
                caps.append("deleteFiles")
            self.keys[new_id] = {
                "secret": f"K005-restricted-secret-{new_id}",
                "caps": caps,
                "buckets": body.get("bucketIds"),
            }
            return 200, {
                "applicationKeyId": new_id,
                "applicationKey": self.keys[new_id]["secret"],
            }
        if name == "b2_delete_key":
            if not self.capable(key_id, "deleteKeys", None):
                return refuse
            if self.keys.pop(body["applicationKeyId"], None) is None:
                return 400, {"code": "bad_request", "message": "key not found"}
            return 200, {}
        if name == "b2_get_upload_url":
            if not self.capable(key_id, "writeFiles", bucket):
                return refuse
            token = self.new_id("upload-token-")
            self.tokens[token] = key_id
            return 200, {
                "uploadUrl": f"{self.url}/upload/{bucket['id']}",
                "authorizationToken": token,
            }
        if name == "b2_hide_file":
            if not self.capable(key_id, "writeFiles", bucket):
                return refuse
            self.add_version(bucket, body["fileName"], "hide")
            return 200, {"action": "hide"}
        if name == "b2_list_file_versions":
            if not self.capable(key_id, "listFiles", bucket):
                return refuse
            prefix = body.get("prefix", "")
            files = [
                self.listed(v)
                for v in sorted(
                    bucket["versions"], key=lambda v: (v["fileName"], -v["at"])
                )
                if v["fileName"].startswith(prefix)
            ]
            return 200, {"files": files, "nextFileName": None, "nextFileId": None}
        if name == "b2_delete_file_version":
            found = [
                (b, v)
                for b in self.buckets.values()
                for v in b["versions"]
                if v["fileId"] == body["fileId"]
            ]
            if not found:
                return 400, {"code": "file_not_present", "message": "no such version"}
            bucket, version = found[0]
            if not self.capable(key_id, "deleteFiles", bucket):
                return refuse
            if version["until"] and version["until"] > self.clock:
                return 401, {
                    "code": "access_denied",
                    "message": "Object Lock is enabled on the file",
                }
            bucket["versions"].remove(version)
            return 200, {"fileId": version["fileId"]}
        if name == "b2_list_unfinished_large_files":
            return 200, {"files": []}
        if name == "b2_delete_bucket":
            if not self.capable(key_id, "deleteBuckets", None):
                return refuse
            if bucket["versions"]:
                return 400, {
                    "code": "cannot_delete_non_empty_bucket",
                    "message": "not empty",
                }
            del self.buckets[bucket["id"]]
            return 200, {"bucketId": bucket["id"]}
        return 400, {"code": "bad_request", "message": f"the fake has no {name}"}

    # --- S3 ------------------------------------------------------------------

    def s3(
        self, method: str, path: str, query: dict, headers, body: bytes, key_id: str
    ):
        bucket_name, _, key = path.lstrip("/").partition("/")
        key = urllib.parse.unquote(key)
        bucket = self.bucket_named(bucket_name)
        self.s3_requests.append((key_id, method, path))
        if bucket is None:
            return s3_error(404, "NoSuchBucket", "no such bucket")
        if method == "POST" and "delete" in query:
            if not headers.get("Content-MD5"):
                return s3_error(400, "InvalidRequest", "Content-MD5 is required")
            names = re.findall(r"<Key>([^<]*)</Key>", body.decode())
            if not self.capable(key_id, "writeFiles", bucket):
                errors = "".join(
                    f"<Error><Key>{n}</Key><Code>AccessDenied</Code></Error>"
                    for n in names
                )
                return 200, f"<DeleteResult>{errors}</DeleteResult>".encode()
            for name in names:
                self.add_version(bucket, name, "hide")
            deleted = "".join(f"<Deleted><Key>{n}</Key></Deleted>" for n in names)
            return 200, f"<DeleteResult>{deleted}</DeleteResult>".encode()
        if method == "DELETE" and "versionId" in query:
            found = [v for v in bucket["versions"] if v["fileId"] == query["versionId"]]
            if not self.capable(key_id, "deleteFiles", bucket):
                return s3_error(403, "AccessDenied", "Access Denied")
            if found and found[0]["until"] and found[0]["until"] > self.clock:
                return s3_error(403, "AccessDenied", "Object Lock")
            if found:
                bucket["versions"].remove(found[0])
            return 204, b""
        if method == "POST" and "uploads" in query:
            if not self.capable(key_id, "writeFiles", bucket):
                return s3_error(403, "AccessDenied", "Access Denied")
            if bucket["locked"] and self.refuse_multipart_in_lock:
                return s3_error(400, "InvalidRequest", LOCK_MESSAGE)
            return (
                200,
                b"<InitiateMultipartUploadResult><UploadId>u1</UploadId></InitiateMultipartUploadResult>",
            )
        if method == "PUT":
            if not self.capable(key_id, "writeFiles", bucket):
                return s3_error(403, "AccessDenied", "Access Denied")
            multipart = headers.get("X-Fake-Multipart") == "1"
            if bucket["locked"] and not multipart and not headers.get("Content-MD5"):
                return s3_error(400, "InvalidRequest", LOCK_MESSAGE)
            self.add_version(bucket, key, "upload", body)
            return 200, b""
        if method == "GET":
            if not self.capable(key_id, "readFiles", bucket):
                return s3_error(403, "AccessDenied", "Access Denied")
            ordered = newest_first(bucket["versions"], key)
            if not ordered or ordered[0]["action"] != "upload":
                return s3_error(404, "NoSuchKey", "no such key")
            return 200, ordered[0]["data"]
        return s3_error(400, "NotImplemented", f"the fake has no {method} {path}")

    def handler(self):
        fake = self

        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def reply(self, status: int, body: bytes, kind: str = "application/json"):
                self.send_response(status)
                self.send_header("Content-Type", kind)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def body(self) -> bytes:
                return self.rfile.read(int(self.headers.get("Content-Length") or 0))

            def route(self, method: str):
                parts = urllib.parse.urlsplit(self.path)
                data = self.body()
                auth = self.headers.get("Authorization", "")
                with fake.lock:
                    if parts.path == "/b2api/v4/b2_authorize_account":
                        return self.authorize(auth)
                    if parts.path.startswith("/b2api/v4/"):
                        key_id = fake.tokens.get(auth)
                        if key_id is None:
                            return self.reply(401, b'{"code": "bad_auth_token"}')
                        status, out = fake.native(
                            parts.path.rsplit("/", 1)[1], key_id, json.loads(data)
                        )
                        return self.reply(status, json.dumps(out).encode())
                    if parts.path.startswith("/upload/"):
                        return self.upload(parts.path.rsplit("/", 1)[1], auth, data)
                    credential = re.search(r"Credential=([^/]+)/", auth)
                    query = dict(
                        urllib.parse.parse_qsl(parts.query, keep_blank_values=True)
                    )
                    status, out = fake.s3(
                        method,
                        parts.path,
                        query,
                        self.headers,
                        data,
                        credential and credential.group(1),
                    )
                    return self.reply(status, out, "application/xml")

            def authorize(self, auth: str):
                key_id, _, secret = (
                    base64.b64decode(auth.removeprefix("Basic "))
                    .decode()
                    .partition(":")
                )
                key = fake.keys.get(key_id)
                if key is None or key["secret"] != secret:
                    return self.reply(
                        401, b'{"code": "unauthorized", "message": "bad key"}'
                    )
                token = fake.new_id("token-")
                fake.tokens[token] = key_id
                allowed = {
                    "capabilities": key["caps"],
                    "buckets": key["buckets"],
                    "namePrefix": None,
                }
                info = {
                    "apiUrl": fake.url,
                    "s3ApiUrl": fake.url,
                    "downloadUrl": fake.url,
                    "allowed": allowed,
                }
                out = {
                    "accountId": "acct1",
                    "authorizationToken": token,
                    "apiInfo": {"storageApi": info},
                }
                return self.reply(200, json.dumps(out).encode())

            def upload(self, bucket_id: str, token: str, data: bytes):
                key_id = fake.tokens.get(token)
                bucket = fake.buckets.get(bucket_id)
                if key_id is None or not fake.capable(key_id, "writeFiles", bucket):
                    return self.reply(401, b'{"code": "unauthorized"}')
                name = urllib.parse.unquote(self.headers["X-Bz-File-Name"])
                version = fake.add_version(bucket, name, "upload", data)
                return self.reply(
                    200,
                    json.dumps(
                        {"fileId": version["fileId"], "fileName": name}
                    ).encode(),
                )

            def do_GET(self):
                self.route("GET")

            def do_POST(self):
                self.route("POST")

            def do_PUT(self):
                self.route("PUT")

            def do_DELETE(self):
                self.route("DELETE")

        return Handler


def newest_first(versions: list[dict], name: str) -> list[dict]:
    return sorted(
        (v for v in versions if v["fileName"] == name), key=lambda v: -v["at"]
    )


def s3_error(status: int, code: str, message: str) -> tuple[int, bytes]:
    return (
        status,
        f"<Error><Code>{code}</Code><Message>{message}</Message></Error>".encode(),
    )

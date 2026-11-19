"""A fake curl for ops/remote/check-secrets.sh's tests: reads its config from stdin
(-K -), notes its argv and config in FAKE_CURL_LOG, and answers the first rule in
FAKE_CURL_RULES (JSON: [[url substring, status, body, headers]]) whose substring is
in the config's url, writing the body to -o and the headers to -D, and printing the
status for -w."""

import json
import os
import sys


def main(argv: list[str]) -> int:
    config = sys.stdin.read() if "-K" in argv else ""
    with open(os.environ["FAKE_CURL_LOG"], "a") as log:
        log.write(json.dumps({"argv": argv, "config": config}) + "\n")
    url = next(
        (
            line.split("=", 1)[1].strip().strip('"')
            for line in config.splitlines()
            if line.startswith("url")
        ),
        "",
    )
    rules = json.loads(open(os.environ["FAKE_CURL_RULES"]).read())
    status, body, headers = 0, "", ""
    for substring, status, body, headers in rules:
        if substring in url:
            break
    else:
        # As real curl does when nothing answers: -w still prints, then exit 6.
        if "-w" in argv:
            print("000", end="")
        return 6
    for flag, text in (("-o", body), ("-D", headers)):
        if flag in argv:
            with open(argv[argv.index(flag) + 1], "w") as out:
                out.write(text)
    print(status, end="")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))

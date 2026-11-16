"""ops/usability-run's reading of the verification mail from the usability mailbox,
which every run of a batch shares: it finds its own address's mail and deletes
only that one, by UID, since another run's delete renumbers the messages.

The mailbox is a fake imaplib.IMAP4_SSL; nothing here reaches a mail server.
"""

import importlib.machinery
import importlib.util
from email.message import EmailMessage
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parents[2] / "ops" / "usability-run"
MAIL = {"USABILITY_MAIL_ADDRESS": "box@example.com", "USABILITY_MAIL_PASSWORD": "x"}


@pytest.fixture(scope="module")
def tool():
    loader = importlib.machinery.SourceFileLoader("usability_run", str(SCRIPT))
    spec = importlib.util.spec_from_loader("usability_run", loader)
    module = importlib.util.module_from_spec(spec)
    loader.exec_module(module)
    return module


def verification(tool, to: str, token: str) -> bytes:
    message = EmailMessage()
    message["Subject"] = tool.SUBJECT
    message["To"] = to
    message.set_content(f"Open https://staging.example/verify?token={token} to verify.")
    return message.as_bytes()


class Mailbox:
    """INBOX as UID -> message. `gone` are UIDs another run deletes after this
    one's search; `headless` are UIDs whose fetch answers with flags only; and
    `noisy` ones answer with another message's flags before the message."""

    def __init__(self, messages: dict[int, bytes], gone=(), headless=(), noisy=()):
        self.messages = dict(messages)
        self.gone, self.headless, self.noisy = set(gone), set(headless), set(noisy)
        self.flagged: set[int] = set()

    def __call__(self, host, port):
        return self

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def login(self, user, password):
        return "OK", [b""]

    def select(self, folder):
        return ("OK" if folder == "INBOX" else "NO"), [b""]

    def uid(self, command, *args):
        if command == "SEARCH":
            return "OK", [b" ".join(str(u).encode() for u in sorted(self.messages))]
        uid = int(args[0])
        if command == "FETCH":
            if uid in self.gone:
                self.messages.pop(uid, None)
                return "OK", [None]
            if uid in self.headless:
                return "OK", [b"1 (FLAGS (\\Seen))"]
            message = [(b"1 (RFC822 {9}", self.messages[uid]), b")"]
            if uid in self.noisy:
                return "OK", [b"3 (FLAGS (\\Deleted))", *message]
            return "OK", message
        assert command == "STORE" and args[1:] == ("+FLAGS", "\\Deleted"), args
        self.flagged.add(uid)
        return "OK", [b""]

    def expunge(self):
        for uid in self.flagged:
            self.messages.pop(uid, None)
        return "OK", [b""]

    def by_sequence_number(self, *args):
        raise AssertionError(
            "a sequence number changes when another run deletes a message"
        )

    search = fetch = store = by_sequence_number


def test_it_returns_its_own_addresss_link_and_deletes_only_that_mail(tool, monkeypatch):
    box = Mailbox(
        {
            7: verification(tool, "box+other-1@example.com", "theirs"),
            9: verification(tool, "box+jason1-ab@example.com", "mine"),
            12: b"Subject: Welcome\r\nTo: box+jason1-ab@example.com\r\n\r\nhello\r\n",
        }
    )
    monkeypatch.setattr(tool.imaplib, "IMAP4_SSL", box)
    assert tool.link_from_imap(MAIL, "box+jason1-ab@example.com") == "mine"
    assert sorted(box.messages) == [7, 12]


def test_a_mail_another_run_deleted_since_the_search_is_skipped(tool, monkeypatch):
    box = Mailbox(
        {
            5: verification(tool, "box+jason1-ab@example.com", "mine"),
            8: verification(tool, "box+other-1@example.com", "theirs"),
            9: verification(tool, "box+other-2@example.com", "theirs"),
        },
        gone=[9],
        headless=[8],
    )
    monkeypatch.setattr(tool.imaplib, "IMAP4_SSL", box)
    assert tool.link_from_imap(MAIL, "box+jason1-ab@example.com") == "mine"
    assert sorted(box.messages) == [8]


def test_a_mail_is_read_past_another_mails_flags_in_the_answer(tool, monkeypatch):
    """Another run's delete can put a line about its own mail first."""
    box = Mailbox(
        {5: verification(tool, "box+jason1-ab@example.com", "mine")}, noisy=[5]
    )
    monkeypatch.setattr(tool.imaplib, "IMAP4_SSL", box)
    assert tool.link_from_imap(MAIL, "box+jason1-ab@example.com") == "mine"


def test_no_mail_for_the_address_is_none_and_deletes_nothing(tool, monkeypatch):
    box = Mailbox({7: verification(tool, "box+other-1@example.com", "theirs")})
    monkeypatch.setattr(tool.imaplib, "IMAP4_SSL", box)
    assert tool.link_from_imap(MAIL, "box+jason1-ab@example.com") is None
    assert sorted(box.messages) == [7]

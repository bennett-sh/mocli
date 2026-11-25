"""Decides whether a push to main publishes a release, and writes its notes.

The version comes from Cargo.toml and the tag is "v" plus that version.

Commits follow "type(scope): description". A dot after the type, as in
"fix.(core): ..." or "feat.: ...", means "include this in the changelog, but do
not publish for it":

* If every commit since the last release is dotted, nothing is published.
* Otherwise a new version creates a release, and an unchanged version moves its
  tag and replaces its files and notes.

The notes list everything since the previous version.
"""

import argparse
import os
import re
import subprocess
import sys
import tomllib
from dataclasses import dataclass
from pathlib import Path

HEADER = re.compile(
    r"^(?P<type>[A-Za-z]+)(?P<dot>\.)?(?:\((?P<scope>[^()]*)\))?(?P<breaking>!)?:\s*(?P<text>.+)$"
)
BREAKING_FOOTER = re.compile(r"^BREAKING[ -]CHANGE:", re.MULTILINE)
VERSION_TAG = re.compile(r"^v(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?$")

# Changelog sections in order, with the commit types each one collects.
SECTIONS = (
    ("Breaking changes", ()),
    ("Features", ("feat",)),
    ("Fixes", ("fix",)),
    ("Performance", ("perf",)),
    ("Refactoring", ("refactor",)),
    ("Documentation", ("docs",)),
    ("Tests", ("test",)),
    ("Build and CI", ("build", "ci")),
    ("Other changes", ()),
)


@dataclass(frozen=True)
class Commit:
    sha: str
    subject: str
    body: str = ""

    @property
    def header(self):
        return HEADER.match(self.subject)

    @property
    def dotted(self) -> bool:
        """Whether this commit asks not to cause a release."""
        return bool(self.header and self.header["dot"])

    @property
    def breaking(self) -> bool:
        return bool(
            (self.header and self.header["breaking"]) or BREAKING_FOOTER.search(self.body)
        )

    @property
    def section(self) -> str:
        if self.breaking:
            return "Breaking changes"
        if self.header:
            kind = self.header["type"].lower()
            for title, types in SECTIONS:
                if kind in types:
                    return title
        return "Other changes"


@dataclass(frozen=True)
class Plan:
    version: str
    publish: bool
    notes: str

    @property
    def tag(self) -> str:
        return f"v{self.version}"

    @property
    def prerelease(self) -> bool:
        return "-" in self.version


def version_key(tag: str):
    """Orders version tags; a pre-release comes before its release."""
    match = VERSION_TAG.match(tag)
    assert match, tag
    major, minor, patch, pre = match.groups()
    parts = tuple(
        (0, int(part), "") if part.isdigit() else (1, 0, part)
        for part in (pre or "").split(".")
    )
    return (int(major), int(minor), int(patch), pre is None, parts)


def git(repository: Path, *arguments: str) -> str:
    return subprocess.run(
        ["git", *arguments],
        cwd=repository,
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
    ).stdout


def commits(repository: Path, since: str | None) -> list[Commit]:
    """Commits up to HEAD that are not in `since`, oldest first, without merges."""
    revisions = f"{since}..HEAD" if since else "HEAD"
    output = git(
        repository,
        "log",
        "--no-merges",
        "--reverse",
        "--format=%h%x1f%s%x1f%b%x1e",
        revisions,
    )
    records = (record.strip("\n") for record in output.split("\x1e"))
    return [Commit(*record.split("\x1f", 2)) for record in records if record]


def escape(text: str) -> str:
    """Keeps "@name" in a commit from notifying someone."""
    return re.sub(r"(?<![\w`])(@[\w/-]+)", r"`\1`", text)


def entry(commit: Commit) -> str:
    header = commit.header
    if not header:
        return f"- {escape(commit.subject)} ({commit.sha})"
    scope = f"**{escape(header['scope'])}:** " if header["scope"] else ""
    # Types without a section of their own keep their name.
    known = any(header["type"].lower() in types for _, types in SECTIONS)
    prefix = "" if known or commit.breaking else f"{header['type']}: "
    return f"- {scope}{escape(prefix + header['text'])} ({commit.sha})"


def changelog(entries: list[Commit]) -> str:
    blocks = []
    for title, _ in SECTIONS:
        lines = [entry(commit) for commit in entries if commit.section == title]
        if lines:
            blocks.append("\n".join([f"## {title}", "", *lines]))
    return "\n\n".join(blocks) if blocks else "No changes listed."


def plan(repository: Path, github: str | None = None) -> Plan:
    manifest = tomllib.loads((repository / "Cargo.toml").read_text(encoding="utf-8"))
    version = manifest["package"]["version"]
    tag = f"v{version}"
    if not VERSION_TAG.match(tag):
        raise SystemExit(f"Cargo.toml has an unsupported version: {version}")

    every_tag = [t for t in git(repository, "tag", "--list", "v*").split() if VERSION_TAG.match(t)]
    merged = git(repository, "tag", "--list", "v*", "--merged", "HEAD").split()
    earlier = [
        t for t in merged if VERSION_TAG.match(t) and version_key(t) < version_key(tag)
    ]
    previous = max(earlier, key=version_key, default=None)
    released = tag in every_tag

    if released and git(repository, "rev-list", "-n", "1", tag) == git(repository, "rev-parse", "HEAD"):
        # A repeated run, for example after publishing failed.
        publish = True
    else:
        new = commits(repository, tag if released else previous)
        publish = any(not commit.dotted for commit in new)

    notes = changelog(commits(repository, previous))
    if github and previous:
        notes += f"\n\n**Full changelog:** https://github.com/{github}/compare/{previous}...{tag}"
    return Plan(version, publish, notes + "\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--repository", help="owner/name, for the comparison link")
    parser.add_argument("--notes", type=Path, required=True, help="where to write the notes")
    arguments = parser.parse_args()

    result = plan(Path.cwd(), arguments.repository)
    arguments.notes.write_text(result.notes, encoding="utf-8")
    outputs = {
        "publish": str(result.publish).lower(),
        "tag": result.tag,
        "prerelease": str(result.prerelease).lower(),
    }
    lines = "".join(f"{name}={value}\n" for name, value in outputs.items())
    if target := os.environ.get("GITHUB_OUTPUT"):
        with open(target, "a", encoding="utf-8") as file:
            file.write(lines)
    sys.stdout.reconfigure(encoding="utf-8")
    print(lines)
    print(result.notes)
    if not result.publish:
        print("Only dotted commits since the last release: nothing to publish.")
    return 0


if __name__ == "__main__":
    sys.exit(main())

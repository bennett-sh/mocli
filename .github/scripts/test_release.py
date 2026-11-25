"""Tests for release.py against real, temporary git repositories."""

import tempfile
import unittest
from pathlib import Path

from release import Commit, changelog, git, plan, version_key


class Repository:
    def __init__(self, directory: str):
        self.path = Path(directory)
        git(self.path, "init", "--quiet", "--initial-branch=main")
        for key, value in (
            ("user.name", "Test"),
            ("user.email", "test@example.com"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ):
            git(self.path, "config", key, value)
        self.version("0.1.0")

    def version(self, version: str):
        (self.path / "Cargo.toml").write_text(
            f'[package]\nname = "mocli"\nversion = "{version}"\n', encoding="utf-8"
        )

    def commit(self, message: str):
        git(self.path, "add", "--all")
        git(self.path, "commit", "--quiet", "--allow-empty", "--message", message)

    def release(self):
        """Publishes what plan() asks for, as the workflow does."""
        result = plan(self.path, "owner/name")
        if result.publish:
            git(self.path, "tag", "--force", result.tag)
        return result


class PlanTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        self.repo = Repository(directory.name)

    def test_first_release_lists_everything(self):
        self.repo.commit("Initial work")
        self.repo.commit("feat(ui): popup")
        result = self.repo.release()
        self.assertTrue(result.publish)
        self.assertEqual(result.tag, "v0.1.0")
        self.assertIn("**ui:** popup", result.notes)
        self.assertIn("- Initial work (", result.notes)
        self.assertNotIn("Full changelog", result.notes)

    def test_unchanged_version_updates_the_release(self):
        self.repo.commit("feat: one")
        self.repo.release()
        self.repo.commit("fix(core): two")
        result = self.repo.release()
        self.assertTrue(result.publish)
        self.assertEqual(result.tag, "v0.1.0")
        # The notes cover the whole version, not only the latest push.
        self.assertIn("one", result.notes)
        self.assertIn("two", result.notes)

    def test_dotted_commits_alone_publish_nothing(self):
        self.repo.commit("feat: one")
        self.repo.release()
        self.repo.commit("fix.(core): quiet fix")
        self.repo.commit("docs.: quiet docs")
        self.assertFalse(self.repo.release().publish)

        self.repo.commit("fix(core): loud fix")
        result = self.repo.release()
        self.assertTrue(result.publish)
        fixes = result.notes.split("## Fixes")[1].split("##")[0]
        self.assertIn("quiet fix", fixes)
        self.assertIn("loud fix", fixes)

    def test_new_version_creates_a_release_since_the_previous_one(self):
        self.repo.commit("feat: old")
        self.repo.release()
        self.repo.version("0.2.0")
        self.repo.commit("chore: bump version")
        self.repo.commit("feat: new")
        result = self.repo.release()
        self.assertTrue(result.publish)
        self.assertEqual(result.tag, "v0.2.0")
        self.assertNotIn("old", result.notes)
        self.assertIn("new", result.notes)
        self.assertIn("compare/v0.1.0...v0.2.0", result.notes)

    def test_new_version_with_only_dotted_commits_waits(self):
        self.repo.commit("feat: old")
        self.repo.release()
        self.repo.version("0.2.0")
        self.repo.commit("chore.: bump version")
        self.assertFalse(self.repo.release().publish)

    def test_rerun_on_the_tagged_commit_publishes_again(self):
        self.repo.commit("feat: one")
        self.repo.release()
        self.assertTrue(self.repo.release().publish)

    def test_merge_commits_do_not_count(self):
        self.repo.commit("feat: one")
        self.repo.release()
        git(self.repo.path, "switch", "--quiet", "--create", "dev")
        self.repo.commit("fix.: quiet")
        git(self.repo.path, "switch", "--quiet", "main")
        git(self.repo.path, "merge", "--quiet", "--no-ff", "--message", "Merge dev", "dev")
        result = self.repo.release()
        self.assertFalse(result.publish)
        self.assertNotIn("Merge dev", result.notes)

    def test_prerelease_versions(self):
        self.repo.version("1.0.0-rc.1")
        self.repo.commit("feat: candidate")
        result = self.repo.release()
        self.assertTrue(result.prerelease)
        self.repo.version("1.0.0")
        self.repo.commit("chore: final")
        final = self.repo.release()
        self.assertFalse(final.prerelease)
        self.assertIn("compare/v1.0.0-rc.1...v1.0.0", final.notes)


class ChangelogTest(unittest.TestCase):
    def test_sections_breaking_changes_and_mentions(self):
        text = changelog(
            [
                Commit("a1", "feat(api)!: remove old call"),
                Commit("b2", "refactor: tidy", "BREAKING CHANGE: renamed"),
                Commit("c3", "ci.(release): upload @types/node"),
                Commit("d4", "style: spacing"),
                Commit("e5", "Update readme"),
                Commit("f6", "FIX: shouting"),
            ]
        )
        self.assertEqual(
            text,
            "## Breaking changes\n\n- **api:** remove old call (a1)\n- tidy (b2)\n\n"
            "## Fixes\n\n- shouting (f6)\n\n"
            "## Build and CI\n\n- **release:** upload `@types/node` (c3)\n\n"
            "## Other changes\n\n- style: spacing (d4)\n- Update readme (e5)",
        )

    def test_dots_mark_only_the_type(self):
        self.assertTrue(Commit("a", "fix.(core): x").dotted)
        self.assertTrue(Commit("a", "fix.: x").dotted)
        for subject in ("fix(core): x.", "fix: x", "fix(core.): x", "Fix. Something"):
            self.assertFalse(Commit("a", subject).dotted, subject)

    def test_versions_sort_numerically(self):
        tags = ["v0.10.0", "v0.9.0", "v1.0.0", "v1.0.0-rc.2", "v1.0.0-rc.10"]
        self.assertEqual(
            sorted(tags, key=version_key),
            ["v0.9.0", "v0.10.0", "v1.0.0-rc.2", "v1.0.0-rc.10", "v1.0.0"],
        )


if __name__ == "__main__":
    unittest.main()

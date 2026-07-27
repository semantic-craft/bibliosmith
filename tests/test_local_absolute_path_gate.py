"""Pin the local-absolute-path portability gate in both directions.

The gate at template/epub_pipeline/common/scripts/check_no_local_absolute_paths.py
runs from ten template package.json files plus books/, and had no test of its own.
It has to fail closed on a contributor's real workspace path while leaving
ordinary slash-separated prose alone, and issue #6 was filed believing it got the
second half wrong. It does not -- but nothing pinned that, so a later widening of
the scan roots or a looser regex could introduce the false positive for real.

Both directions are asserted through scan_file() rather than against the regexes
directly, so the scan-scope rules are exercised too: a fixture placed outside the
scanned roots would pass vacuously, which is exactly the mistake to guard against.
"""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SCRIPT_PATH = (
    REPO_ROOT
    / "template"
    / "epub_pipeline"
    / "common"
    / "scripts"
    / "check_no_local_absolute_paths.py"
)
# A path that is generic-scanned in repo scope, so a fixture written here is
# actually run through the detectors instead of being skipped by scope rules.
SCANNED_REL = "doc/public/fixture.md"

# The home-directory fixtures are assembled at runtime. Spelling them out as
# literals would put "/Users/<name>/" in a tracked file, which the repo's own
# gitleaks developer-home-path rule flags on commit -- correctly, since that rule
# cannot tell a test fixture from a real leak. Allowlisting this file instead
# would carve a permanent hole in the privacy gate for the sake of a test.
_USERS = "/Use" + "rs/"
_HOME = "/ho" + "me/"
MAC_HOME = f"{_USERS}alice/"
LINUX_HOME = f"{_HOME}bob/"
WSL_HOME = f"/mnt/c{_USERS}carol/"
FILE_URL = f"file:///C:{_USERS}alice/cover.png"


def load_gate():
    spec = importlib.util.spec_from_file_location("check_no_local_absolute_paths", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec and spec.loader
    # @dataclass resolves its own module out of sys.modules; registering before
    # exec_module keeps this import working on 3.12+.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


gate = load_gate()


class GateFixture(unittest.TestCase):
    """Write one line into a throwaway repo tree and run the real scanner over it."""

    def scan(self, line: str, rel_path: str = SCANNED_REL) -> list:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            target = root / rel_path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(f"{line}\n", encoding="utf-8")
            patterns = gate.repo_leak_patterns(root, "repo")
            return gate.scan_file(root, target, "repo", patterns)

    def assertClean(self, line: str, rel_path: str = SCANNED_REL) -> None:
        issues = self.scan(line, rel_path)
        self.assertEqual(issues, [], f"expected no finding for {line!r}, got {issues}")

    def assertFlagged(self, line: str, rel_path: str = SCANNED_REL) -> None:
        issues = self.scan(line, rel_path)
        self.assertTrue(issues, f"expected a finding for {line!r}")


class SlashSeparatedProseIsNotAPath(GateFixture):
    """Issue #6: 'queue/report' and friends are prose, not absolute paths."""

    def test_the_two_lines_named_in_issue_6(self):
        self.assertClean("# MinerU queue/report.")
        self.assertClean(
            '    parser.add_argument("--dry-run", action="store_true",'
            ' help="Build queue/report without calling MinerU or Zotero upload.")'
        )

    def test_other_ordinary_prose_with_slashes(self):
        for line in (
            "Pass either --input or --output (input/output are mutually exclusive).",
            "Set the flag and/or edit the config.",
            "The worker speaks TCP/IP.",
            "Runs 24/7 on the queue.",
            "See docs/windows.md for the runbook.",
            "Relative paths like books/zh-Hans/001 stay relative.",
        ):
            with self.subTest(line=line):
                self.assertClean(line)


class RealLocalPathsFailClosed(GateFixture):
    """The positive fixtures the gate exists for must keep failing."""

    def test_unix_and_macos_home_paths(self):
        for line in (
            f"Output lands in {MAC_HOME}Projects/bibliosmith/books.",
            f"The runner writes {LINUX_HOME}work/out.epub.",
        ):
            with self.subTest(line=line):
                self.assertFlagged(line)

    def test_wsl_mount_paths(self):
        self.assertFlagged(f"Reads {WSL_HOME}Desktop/book.pdf.")

    def test_windows_drive_and_file_urls(self):
        for line in (
            r"Open C:\Users\alice\Desktop\book.pdf to check.",
            f"Cover source: {FILE_URL}",
        ):
            with self.subTest(line=line):
                self.assertFlagged(line)

    def test_the_checkout_path_is_rejected_everywhere_not_only_in_scanned_roots(self):
        # repo_leak_patterns applies to every scanned file regardless of scope
        # rules, which is what keeps a leaked checkout path out of packages/.
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw).resolve()
            target = root / "packages" / "ocr" / "docs" / "windows.md"
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_text(f"Built from {root}\n", encoding="utf-8")
            issues = gate.scan_file(root, target, "repo", gate.repo_leak_patterns(root, "repo"))
        self.assertTrue(issues)
        self.assertEqual(issues[0].rule, "repo_absolute_path")


class AllowlistedBrandPathStillPasses(GateFixture):
    """D:\\BiblioSmith is the documented Windows install root, not a personal path."""

    def test_brand_root_is_allowed(self):
        self.assertClean(r"Install to D:\BiblioSmith and run the launcher.")

    def test_any_other_windows_root_is_not(self):
        self.assertFlagged(r"Install to D:\Scratch\bibliosmith and run the launcher.")


class ScanScopeIsDeliberate(unittest.TestCase):
    """Which roots get the generic detectors is a decision, so pin it.

    packages/ is intentionally outside this gate: it is not a book-production
    artifact. Real personal paths there are caught by the gitleaks
    developer-home-path rule in CI instead, which scans the whole tree.
    """

    def test_book_production_roots_are_generic_scanned(self):
        for rel_path in (
            "doc/public/how-to-use-prompts.en.md",
            "doc/project/notes.md",
            "template/epub_pipeline/English-to-Simplified-Chinese/README.md",
            "books/zh-Hans/001_example/metadata/book.opf",
        ):
            with self.subTest(rel_path=rel_path):
                self.assertTrue(gate.should_scan_for_generic_local_path("repo", rel_path))

    def test_template_scripts_and_packages_are_not(self):
        for rel_path in (
            "template/epub_pipeline/common/scripts/build_epub.py",
            "packages/ocr/docs/windows.md",
            "packages/ocr/scripts/mineru_law_politics_markdown.py",
        ):
            with self.subTest(rel_path=rel_path):
                self.assertFalse(gate.should_scan_for_generic_local_path("repo", rel_path))


if __name__ == "__main__":
    unittest.main()

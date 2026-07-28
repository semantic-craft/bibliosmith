"""Pin the split gitleaks personal-info scan (issue #77).

The secrets config must keep ``useDefault`` for the full default ruleset. That
same inheritance silently drops every ``/home/...`` match, so home-path and
private-network rules live in ``.gitleaks-personal.toml`` and run as a second
pass over the tip tree. Fixtures assemble path strings at runtime so this file
does not itself trip the gate it is testing.
"""

from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
SECRETS_CONFIG = REPO_ROOT / ".gitleaks.toml"
PERSONAL_CONFIG = REPO_ROOT / ".gitleaks-personal.toml"

_USERS = "/Use" + "rs/"
_HOME = "/ho" + "me/"
MAC_HOME = f"{_USERS}alice/"
LINUX_HOME = f"{_HOME}bob/"
RUNNER_HOME = f"{_HOME}runner/"


def _gitleaks_available() -> bool:
    return shutil.which("gitleaks") is not None


@unittest.skipUnless(_gitleaks_available(), "gitleaks binary not on PATH")
class GitleaksPersonalInfoScan(unittest.TestCase):
    def scan_dir(self, config: Path, tree: Path) -> list[dict]:
        report = tree / "report.json"
        completed = subprocess.run(
            [
                "gitleaks",
                "dir",
                str(tree),
                "--config",
                str(config),
                "--no-banner",
                "--report-format",
                "json",
                "--report-path",
                str(report),
            ],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertIn(
            completed.returncode,
            (0, 1),
            msg=f"gitleaks failed: {completed.stderr or completed.stdout}",
        )
        if not report.exists() or report.stat().st_size == 0:
            return []
        return json.loads(report.read_text(encoding="utf-8"))

    def test_personal_config_flags_both_home_layouts(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            tree = Path(raw)
            (tree / "probe.txt").write_text(
                f"mac {MAC_HOME}Projects/x.md\n"
                f"linux {LINUX_HOME}Projects/x.md\n"
                f"runner {RUNNER_HOME}work/ci/x\n",
                encoding="utf-8",
            )
            findings = self.scan_dir(PERSONAL_CONFIG, tree)

        matches = {item.get("Match") for item in findings}
        self.assertIn(MAC_HOME, matches)
        self.assertIn(LINUX_HOME, matches)
        self.assertNotIn(RUNNER_HOME, matches)
        self.assertTrue(
            all(item.get("RuleID") == "developer-home-path" for item in findings),
            findings,
        )

    def test_secrets_config_does_not_carry_home_path_rules(self) -> None:
        """Home paths are the personal pass's job; secrets stay useDefault-only."""
        with tempfile.TemporaryDirectory() as raw:
            tree = Path(raw)
            (tree / "probe.txt").write_text(
                f"mac {MAC_HOME}Projects/x.md\n"
                f"linux {LINUX_HOME}Projects/x.md\n",
                encoding="utf-8",
            )
            findings = self.scan_dir(SECRETS_CONFIG, tree)

        matches = {item.get("Match") for item in findings}
        self.assertNotIn(LINUX_HOME, matches)
        self.assertNotIn(MAC_HOME, matches)

    def test_tip_tree_is_clean_under_both_configs(self) -> None:
        """Materialise HEAD and scan it — same shape as pre-push personal scan."""
        with tempfile.TemporaryDirectory() as raw:
            tip = Path(raw)
            archive = subprocess.run(
                ["git", "archive", "HEAD"],
                cwd=REPO_ROOT,
                check=True,
                capture_output=True,
            )
            subprocess.run(
                ["tar", "-x", "-C", str(tip)],
                input=archive.stdout,
                check=True,
            )
            for config in (SECRETS_CONFIG, PERSONAL_CONFIG):
                with self.subTest(config=config.name):
                    # Config files live at repo root; copy so path allowlists apply.
                    shutil.copy2(config, tip / config.name)
                    findings = self.scan_dir(tip / config.name, tip)
                    self.assertEqual(
                        findings,
                        [],
                        f"{config.name} flagged: "
                        f"{[(i.get('File'), i.get('Match')) for i in findings[:10]]}",
                    )


if __name__ == "__main__":
    unittest.main()

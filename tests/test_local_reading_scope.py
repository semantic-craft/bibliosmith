import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]

REMOVED_WORKFLOW_PATHS = [
    "template/epub_pipeline",
    "skills/public-domain-epub-pipeline",
    "books/private",
    "books/zh-Hans",
    "books/scripts/create_book_project.py",
    "doc/public",
    "research/English-to-Simplified-Chinese/book-discovery",
]


def test_repository_does_not_ship_the_upstream_publishing_workflow() -> None:
    present = [
        relative
        for relative in REMOVED_WORKFLOW_PATHS
        if (REPO_ROOT / relative).exists()
    ]

    assert present == []


def test_legacy_private_projects_remain_ignored() -> None:
    completed = subprocess.run(
        [
            "git",
            "check-ignore",
            "--no-index",
            "--quiet",
            "books/private/legacy-project/source/original.pdf",
        ],
        cwd=REPO_ROOT,
        check=False,
    )

    assert completed.returncode == 0


def test_opencode_loads_only_the_local_reading_entrypoints() -> None:
    config = (REPO_ROOT / "opencode.jsonc").read_text(encoding="utf-8")

    assert "skills/local-book-reading-pipeline/SKILL.md" in config
    assert "template/epub_pipeline" not in config
    assert "public-domain-epub-pipeline" not in config

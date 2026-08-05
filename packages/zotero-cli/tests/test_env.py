import os
from pathlib import Path

from zotero_cli.root_env import load_root_dotenv


def test_load_root_dotenv_uses_repo_root_and_preserves_process_env(
    monkeypatch, tmp_path: Path
) -> None:  # type: ignore[no-untyped-def]
    repo_root = tmp_path
    package_root = repo_root / "packages" / "zotero-cli"
    package_root.mkdir(parents=True)
    (repo_root / "pyproject.toml").write_text("[tool.uv.workspace]\n", encoding="utf-8")
    (repo_root / ".env").write_text(
        "ROOT_ENV_TEST=from-root\nENV_PRECEDENCE_TEST=from-file\nBLANK_ENV_TEST=\n",
        encoding="utf-8",
    )
    (package_root / ".env").write_text(
        "ROOT_ENV_TEST=from-package\nPACKAGE_ONLY_TEST=from-package\n",
        encoding="utf-8",
    )
    monkeypatch.delenv("ROOT_ENV_TEST", raising=False)
    monkeypatch.delenv("PACKAGE_ONLY_TEST", raising=False)
    monkeypatch.delenv("BLANK_ENV_TEST", raising=False)
    monkeypatch.setenv("ENV_PRECEDENCE_TEST", "from-process")

    load_root_dotenv(package_root)

    assert os.environ["ROOT_ENV_TEST"] == "from-root"
    assert os.environ["ENV_PRECEDENCE_TEST"] == "from-process"
    assert "PACKAGE_ONLY_TEST" not in os.environ
    assert "BLANK_ENV_TEST" not in os.environ


def test_load_root_dotenv_silently_skips_standalone_install(
    monkeypatch, tmp_path: Path
) -> None:  # type: ignore[no-untyped-def]
    monkeypatch.delenv("STANDALONE_ENV_TEST", raising=False)

    load_root_dotenv(tmp_path)

    assert "STANDALONE_ENV_TEST" not in os.environ


def test_desktop_runtime_can_disable_dotenv_loading(
    monkeypatch, tmp_path: Path
) -> None:  # type: ignore[no-untyped-def]
    (tmp_path / "pyproject.toml").write_text("[tool.uv.workspace]\n", encoding="utf-8")
    (tmp_path / "packages").mkdir()
    (tmp_path / ".env").write_text("DESKTOP_SECRET=must-not-load\n", encoding="utf-8")
    monkeypatch.setenv("BIBLIOSMITH_DISABLE_DOTENV", "1")
    monkeypatch.delenv("DESKTOP_SECRET", raising=False)

    load_root_dotenv(tmp_path)

    assert "DESKTOP_SECRET" not in os.environ

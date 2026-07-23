from __future__ import annotations

import importlib.util
import os
from pathlib import Path
import sys
import unittest
from unittest import mock


PACKAGE_ROOT = Path(__file__).resolve().parents[1]
ENTRYPOINTS = (
    PACKAGE_ROOT / "mineru.py",
    PACKAGE_ROOT / "paddle.py",
    PACKAGE_ROOT / "scripts" / "paddleocr_vl_cli.py",
    PACKAGE_ROOT / "scripts" / "pdf_to_html_paddleocr.py",
    PACKAGE_ROOT / "scripts" / "zotero_llm_worker.py",
)


def load_module(path: Path, index: int):  # type: ignore[no-untyped-def]
    module_name = f"ocr_root_env_test_{index}"
    spec = importlib.util.spec_from_file_location(module_name, path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"Cannot import {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[module_name] = module
    spec.loader.exec_module(module)
    return module


class RootEnvTests(unittest.TestCase):
    def test_python_entrypoints_load_only_root_env_and_preserve_process_env(self) -> None:
        with self.subTest("all OCR entrypoints"):
            from tempfile import TemporaryDirectory

            with TemporaryDirectory() as tmp:
                repo_root = Path(tmp)
                package_root = repo_root / "packages" / "ocr"
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

                modules = [load_module(path, index) for index, path in enumerate(ENTRYPOINTS)]
                for module in modules:
                    with self.subTest(entrypoint=module.__file__), mock.patch.dict(os.environ, {}, clear=False):
                        os.environ.pop("ROOT_ENV_TEST", None)
                        os.environ.pop("PACKAGE_ONLY_TEST", None)
                        os.environ.pop("BLANK_ENV_TEST", None)
                        os.environ["ENV_PRECEDENCE_TEST"] = "from-process"

                        module.load_root_dotenv(package_root)

                        self.assertEqual("from-root", os.environ.get("ROOT_ENV_TEST"))
                        self.assertEqual("from-process", os.environ.get("ENV_PRECEDENCE_TEST"))
                        self.assertNotIn("PACKAGE_ONLY_TEST", os.environ)
                        self.assertNotIn("BLANK_ENV_TEST", os.environ)


if __name__ == "__main__":
    unittest.main()

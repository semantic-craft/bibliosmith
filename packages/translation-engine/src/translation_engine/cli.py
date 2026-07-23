import argparse
import json
from pathlib import Path
from typing import Sequence

from .engine import EngineError, run_manifest


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Translate Markdown chapter units")
    parser.add_argument("--manifest", required=True, type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        report = run_manifest(args.manifest)
    except (OSError, json.JSONDecodeError, EngineError) as error:
        code = error.code if isinstance(error, EngineError) else "invalid_manifest"
        report = {
            "schema": "translation-engine-report-v1",
            "summary": {"total": 0, "completed": 0, "failed": 0},
            "units": [],
            "error": {"code": code},
        }
        print(json.dumps(report, separators=(",", ":")))
        return 2

    print(json.dumps(report, separators=(",", ":")))
    return 0 if report["summary"]["failed"] == 0 else 1

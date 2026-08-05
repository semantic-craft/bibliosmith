import argparse
import json
from pathlib import Path
from typing import Sequence

from .engine import EngineError
from .prompt_preview import run_prompt_preview_manifest


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Compile a translation prompt preview")
    parser.add_argument("--manifest", required=True, type=Path)
    args = parser.parse_args(argv)
    try:
        report = run_prompt_preview_manifest(args.manifest)
    except (OSError, json.JSONDecodeError, EngineError) as error:
        code = error.code if isinstance(error, EngineError) else "invalid_manifest"
        print(json.dumps({"error": {"code": code}}, separators=(",", ":")))
        return 2
    print(json.dumps(report, ensure_ascii=False, separators=(",", ":")))
    return 0

import argparse
import json
from pathlib import Path
from typing import Sequence

from .engine import EngineError
from .providers import ProviderError
from .sample import run_sample_manifest


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Sample Markdown translation units")
    parser.add_argument("--manifest", required=True, type=Path)
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        report = run_sample_manifest(args.manifest)
    except (
        OSError,
        json.JSONDecodeError,
        EngineError,
        ProviderError,
        ValueError,
    ) as error:
        code = (
            error.code
            if isinstance(error, (EngineError, ProviderError))
            else "invalid_manifest"
        )
        report = {
            "schema": "translation-engine-sample-report-v1",
            "samples": [],
            "error": {"code": code},
        }
        print(json.dumps(report, separators=(",", ":")))
        return 2

    print(json.dumps(report, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

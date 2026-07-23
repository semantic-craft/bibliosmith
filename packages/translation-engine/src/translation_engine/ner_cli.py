import argparse
import json
from pathlib import Path
from typing import Sequence

from .ner import ProviderFactory, extract_ner_candidates
from .providers import ProviderUnavailableError, create_provider


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description=(
            "Sample local source text and write NER glossary candidates for manual review"
        )
    )
    parser.add_argument("--project-root", required=True, type=Path)
    parser.add_argument("--provider-profile-id", required=True)
    parser.add_argument("--provider-config-id", required=True)
    return parser


def main(
    argv: Sequence[str] | None = None,
    *,
    provider_factory: ProviderFactory = create_provider,
) -> int:
    args = build_parser().parse_args(argv)
    try:
        report = extract_ner_candidates(
            args.project_root,
            provider_profile_id=args.provider_profile_id,
            provider_config_id=args.provider_config_id,
            provider_factory=provider_factory,
        )
    except (OSError, UnicodeDecodeError, ValueError, ProviderUnavailableError) as error:
        code = getattr(error, "code", "ner_failed")
        print(
            json.dumps(
                {
                    "schema": "translation-engine-ner-report-v1",
                    "error": {"code": code},
                },
                separators=(",", ":"),
            )
        )
        return 2

    print(json.dumps(report, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

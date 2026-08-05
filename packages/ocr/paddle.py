#!/usr/bin/env python3
"""Baidu AI Studio PaddleOCR-VL 1.6 command line client.

The CLI reads BAIDU_PADDLEOCR_TOKEN from the process environment or the
monorepo-root .env file, submits one local file or URL, polls the async job,
and writes Markdown plus referenced images to a local output directory.
"""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path
from urllib.parse import urlparse

import requests


APP_ROOT = Path(__file__).resolve().parent
DEFAULT_JOB_URL = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs"
DEFAULT_MODEL = "PaddleOCR-VL-1.6"
DEFAULT_OPTIONAL_PAYLOAD = {
    "useDocOrientationClassify": False,
    "useDocUnwarping": False,
    "useChartRecognition": False,
}


class PaddleOCRError(Exception):
    pass


def load_dotenv(path: Path) -> None:
    if not path.exists():
        return
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, value = line.split("=", 1)
        key = key.strip()
        value = value.strip().strip('"').strip("'")
        if key and value and key not in os.environ:
            os.environ[key] = value


def load_root_dotenv(start: Path = APP_ROOT) -> None:
    if os.environ.get("BIBLIOSMITH_DISABLE_DOTENV") == "1":
        return
    for candidate in (start.resolve(), *start.resolve().parents):
        if (candidate / "pyproject.toml").is_file() and (candidate / "packages").is_dir():
            load_dotenv(candidate / ".env")
            return


def token_present() -> bool:
    return bool(os.environ.get("BAIDU_PADDLEOCR_TOKEN", "").strip())


def safe_name(value: str) -> str:
    cleaned = "".join("_" if ch in '\\/:*?"<>|\x00' else ch for ch in value).strip()
    return cleaned or "document"


def is_url(value: str) -> bool:
    parsed = urlparse(value)
    return parsed.scheme in {"http", "https"}


def checked_json(response: requests.Response, phase: str) -> dict:
    if response.status_code != 200:
        raise PaddleOCRError(f"{phase} failed: HTTP {response.status_code}: {response.text[:1000]}")
    try:
        return response.json()
    except json.JSONDecodeError as exc:
        raise PaddleOCRError(f"{phase} returned non-JSON response: {response.text[:1000]}") from exc


def submit_job(args: argparse.Namespace, headers: dict[str, str], optional_payload: dict) -> str:
    input_value = args.input
    if is_url(input_value):
        payload = {
            "fileUrl": input_value,
            "model": args.model,
            "optionalPayload": optional_payload,
        }
        response = requests.post(
            args.job_url,
            json=payload,
            headers={**headers, "Content-Type": "application/json"},
            timeout=args.timeout_seconds,
        )
    else:
        file_path = Path(input_value).expanduser().resolve()
        if not file_path.exists():
            raise PaddleOCRError(f"File not found: {file_path}")
        data = {
            "model": args.model,
            "optionalPayload": json.dumps(optional_payload),
        }
        with file_path.open("rb") as fh:
            files = {"file": (file_path.name, fh)}
            response = requests.post(
                args.job_url,
                headers=headers,
                data=data,
                files=files,
                timeout=args.timeout_seconds,
            )
    payload = checked_json(response, "submit")
    try:
        return payload["data"]["jobId"]
    except KeyError as exc:
        raise PaddleOCRError(f"submit response missing jobId: {payload}") from exc


def poll_json_url(args: argparse.Namespace, headers: dict[str, str], job_id: str) -> str:
    deadline = time.time() + args.max_runtime_seconds
    while time.time() < deadline:
        response = requests.get(f"{args.job_url}/{job_id}", headers=headers, timeout=args.timeout_seconds)
        payload = checked_json(response, "poll")
        data = payload.get("data", {})
        state = data.get("state")
        if state == "pending":
            print("state=pending", flush=True)
        elif state == "running":
            progress = data.get("extractProgress", {})
            print(
                "state=running extracted={}/{}".format(
                    progress.get("extractedPages", "?"),
                    progress.get("totalPages", "?"),
                ),
                flush=True,
            )
        elif state == "done":
            progress = data.get("extractProgress", {})
            print(
                "state=done extracted={} start={} end={}".format(
                    progress.get("extractedPages", "?"),
                    progress.get("startTime", "?"),
                    progress.get("endTime", "?"),
                ),
                flush=True,
            )
            try:
                return data["resultUrl"]["jsonUrl"]
            except KeyError as exc:
                raise PaddleOCRError(f"done response missing jsonUrl: {payload}") from exc
        elif state == "failed":
            raise PaddleOCRError(f"job failed: {data.get('errorMsg')}")
        else:
            raise PaddleOCRError(f"unexpected job state: {state}")
        time.sleep(args.poll_seconds)
    raise PaddleOCRError(f"Timed out after {args.max_runtime_seconds}s waiting for {job_id}")


def download_jsonl(json_url: str, timeout_seconds: int) -> str:
    response = requests.get(json_url, timeout=timeout_seconds)
    response.raise_for_status()
    return response.text


def write_outputs(jsonl_text: str, output_dir: Path, timeout_seconds: int) -> int:
    output_dir.mkdir(parents=True, exist_ok=True)
    (output_dir / "result.jsonl").write_text(jsonl_text, encoding="utf-8")
    page_num = 0
    for line_num, raw in enumerate(jsonl_text.splitlines(), start=1):
        line = raw.strip()
        if not line:
            continue
        payload = json.loads(line)
        result = payload.get("result", payload)
        for res in result.get("layoutParsingResults", []):
            md_path = output_dir / f"doc_{page_num}.md"
            markdown = res.get("markdown", {})
            md_path.write_text(markdown.get("text", ""), encoding="utf-8")
            print(f"markdown={md_path}")

            for img_path, img_url in (markdown.get("images") or {}).items():
                local_path = output_dir / safe_name(img_path)
                local_path.parent.mkdir(parents=True, exist_ok=True)
                img_response = requests.get(img_url, timeout=timeout_seconds)
                img_response.raise_for_status()
                local_path.write_bytes(img_response.content)
                print(f"image={local_path}")

            for img_name, img_url in (res.get("outputImages") or {}).items():
                filename = output_dir / f"{safe_name(img_name)}_{page_num}.jpg"
                img_response = requests.get(img_url, timeout=timeout_seconds)
                img_response.raise_for_status()
                filename.write_bytes(img_response.content)
                print(f"image={filename}")

            page_num += 1
    return page_num


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Submit PDFs/images to Baidu PaddleOCR-VL 1.6 and save Markdown.")
    parser.add_argument("input", nargs="?", help="Local file path or HTTP(S) file URL.")
    parser.add_argument("-o", "--output-dir", default="output/paddle")
    parser.add_argument("--model", default=os.environ.get("BAIDU_PADDLEOCR_MODEL", DEFAULT_MODEL))
    parser.add_argument("--job-url", default=os.environ.get("BAIDU_PADDLEOCR_JOB_URL", DEFAULT_JOB_URL))
    parser.add_argument("--poll-seconds", type=int, default=int(os.environ.get("BAIDU_POLL_SECONDS", "5")))
    parser.add_argument("--timeout-seconds", type=int, default=int(os.environ.get("REQUEST_TIMEOUT_SECONDS", "120")))
    parser.add_argument("--max-runtime-seconds", type=int, default=1800)
    parser.add_argument("--use-doc-orientation-classify", action="store_true")
    parser.add_argument("--use-doc-unwarping", action="store_true")
    parser.add_argument("--use-chart-recognition", action="store_true")
    parser.add_argument("--self-test", action="store_true", help="Check config/imports without submitting a job.")
    return parser


def main(argv: list[str] | None = None) -> int:
    load_root_dotenv()
    args = build_parser().parse_args(argv)
    if args.self_test:
        print(f"model={args.model}")
        print(f"job_url={args.job_url}")
        print(f"token={'present' if token_present() else 'missing'}")
        print("requests=present")
        return 0 if token_present() and args.model == DEFAULT_MODEL else 1
    if not args.input:
        raise SystemExit("input is required unless --self-test is used")
    token = os.environ.get("BAIDU_PADDLEOCR_TOKEN", "").strip()
    if not token:
        raise PaddleOCRError("BAIDU_PADDLEOCR_TOKEN is not configured")
    optional_payload = dict(DEFAULT_OPTIONAL_PAYLOAD)
    optional_payload["useDocOrientationClassify"] = bool(args.use_doc_orientation_classify)
    optional_payload["useDocUnwarping"] = bool(args.use_doc_unwarping)
    optional_payload["useChartRecognition"] = bool(args.use_chart_recognition)

    headers = {"Authorization": f"bearer {token}"}
    print(f"model={args.model}")
    print(f"submitting={args.input}")
    job_id = submit_job(args, headers, optional_payload)
    print(f"job_id={job_id}")
    json_url = poll_json_url(args, headers, job_id)
    jsonl_text = download_jsonl(json_url, args.timeout_seconds)
    pages = write_outputs(jsonl_text, Path(args.output_dir).resolve(), args.timeout_seconds)
    print(f"pages={pages}")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except PaddleOCRError as exc:
        print(f"error: {exc}", file=sys.stderr)
        raise SystemExit(1)

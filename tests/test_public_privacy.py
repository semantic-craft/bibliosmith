import hashlib
import re
import subprocess
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[1]
PRIVATE_HOST_HASHES = {
    # Keep only the digest: the historical machine identifier must not be
    # republished by the guard that prevents it from returning.
    "1614ceeec50a9336ebf690886caa747d6811c45d37086a3fa7b11c9e83926c6c",
}
TOKEN = re.compile(rb"(?<![A-Za-z0-9_-])[A-Za-z][A-Za-z0-9_-]{2,63}(?![A-Za-z0-9_-])")


def test_tracked_tree_excludes_private_host_identifiers():
    tracked = subprocess.run(
        ["git", "ls-files", "-z"],
        cwd=REPO_ROOT,
        check=True,
        capture_output=True,
    ).stdout.split(b"\0")

    for raw_path in filter(None, tracked):
        relative_path = raw_path.decode("utf-8")
        data = (REPO_ROOT / relative_path).read_bytes()
        for token in TOKEN.findall(data):
            digest = hashlib.sha256(token.lower()).hexdigest()
            assert digest not in PRIVATE_HOST_HASHES, (
                f"public tree contains a private host identifier in {relative_path}"
            )

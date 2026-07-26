import hashlib
import json
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from xml.etree import ElementTree as ET


REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "packages"))

from digest.bibliosmith_digest.core import run_digest


NS = {
    "opf": "http://www.idpf.org/2007/opf",
    "xhtml": "http://www.w3.org/1999/xhtml",
}


class BiblioSmithDigestTest(unittest.TestCase):
    def test_missing_config_auto_generates_sidecar_for_long_books(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(
                root,
                title="长篇小说示例",
                chapters=[
                    ("第一章", "故事从漫长旅程开始，主角面对家族、城市和时代变化。"),
                    ("第二章", "冲突扩大，人物关系和核心问题逐步交织。"),
                    ("第三章", "旧秩序瓦解，新选择迫使主角重新理解世界。"),
                    ("第四章", "结局回应开端，也留下关于命运和责任的思考。"),
                ],
            )

            result = run_digest(root)

            self.assertEqual(result["status"], "PASS")
            self.assertEqual(result["auto_decision"], "generated")
            self.assertFalse(result["merged"])
            self.assertTrue((root / "output" / "digest" / "digest.xhtml").exists())
            self.assertFalse((root / "output" / "book_digest.epub").exists())

    def test_missing_config_auto_skips_short_stories_and_natural_science(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root, title="自然科学短篇读本")

            result = run_digest(root)

            self.assertEqual(result["status"], "SKIPPED")
            self.assertEqual(result["reason"], "auto_policy_excluded")
            self.assertFalse((root / "output" / "digest").exists())

    def test_disabled_config_leaves_epub_untouched(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            epub = create_minimal_epub(root)
            before = sha256(epub)
            write_config(root, {"enabled": False})

            result = run_digest(root)

            self.assertEqual(result["status"], "SKIPPED")
            self.assertEqual(result["reason"], "disabled")
            self.assertEqual(sha256(epub), before)
            self.assertFalse((root / "output" / "digest").exists())

    def test_enabled_config_can_generate_sidecar_without_merging(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root)
            write_config(root, {"enabled": True, "merge_into_epub": False})

            result = run_digest(root)

            self.assertEqual(result["status"], "PASS")
            self.assertEqual(result["merged"], False)
            digest_xhtml = root / "output" / "digest" / "digest.xhtml"
            report = json.loads((root / "qa" / "digest" / "digest_report.json").read_text("utf-8"))
            self.assertTrue(digest_xhtml.exists())
            digest_text = digest_xhtml.read_text("utf-8")
            self.assertIn("全书导读", digest_text)
            self.assertIn("<svg", digest_text)
            self.assertIn("第一章", digest_text)
            state = json.loads((root / "output" / "digest" / "digest_state.json").read_text("utf-8"))
            self.assertEqual(state["topology"]["nodes"][0]["title"], "第一章")
            self.assertEqual(state["topology"]["edges"], [])
            self.assertIn("knowledge_graph", state)
            self.assertIn("agent_packet_manifest", state)
            self.assertTrue((root / "output" / "digest" / "knowledge_map.svg").exists())
            self.assertTrue((root / "output" / "digest" / "agent_packets" / "000_digest_generation.md").exists())
            self.assertTrue((root / "qa" / "digest" / "digest_review_checklist.md").exists())
            self.assertIn("章节拓扑", digest_text)
            self.assertIn("知识脉络图", digest_text)
            self.assertEqual(report["status"], "PASS")
            self.assertFalse((root / "output" / "book_digest.epub").exists())

    def test_enabled_config_can_merge_digest_as_epub_chapter(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root)
            write_config(root, {"enabled": True, "merge_into_epub": True})

            result = run_digest(root)

            merged = root / "output" / "book_digest.epub"
            self.assertEqual(result["status"], "PASS")
            self.assertEqual(result["merged"], True)
            self.assertTrue(merged.exists())
            self.assert_epub_contains_digest(merged)

    def test_config_language_controls_digest_xhtml_language(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root)
            write_config(root, {"enabled": True, "merge_into_epub": False, "language": "en"})

            run_digest(root)

            digest_text = (root / "output" / "digest" / "digest.xhtml").read_text("utf-8")
            self.assertIn('xml:lang="en"', digest_text)
            self.assertIn('lang="en"', digest_text)

    def test_merge_rejects_output_path_that_overwrites_source_epub(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root)
            write_config(
                root,
                {
                    "enabled": True,
                    "merge_into_epub": True,
                    "source_epub": "output/book.epub",
                    "output_epub": "output/book.epub",
                },
            )

            with self.assertRaises(ValueError):
                run_digest(root)

    def test_module_cli_runs_against_book_root_config(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            create_minimal_epub(root)
            write_config(root, {"enabled": True, "merge_into_epub": False})

            completed = subprocess.run(
                [
                    sys.executable,
                    "-m",
                    "digest.bibliosmith_digest",
                    "--book-root",
                    str(root),
                ],
                cwd=REPO_ROOT,
                check=False,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

            self.assertEqual(completed.returncode, 0, completed.stderr)
            self.assertIn('"status": "PASS"', completed.stdout)
            self.assertTrue((root / "output" / "digest" / "digest.xhtml").exists())

    def assert_epub_contains_digest(self, epub):
        with zipfile.ZipFile(epub) as archive:
            names = set(archive.namelist())
            self.assertIn("EPUB/text/bibliosmith-digest.xhtml", names)
            digest_xhtml = archive.read("EPUB/text/bibliosmith-digest.xhtml").decode("utf-8")
            package = ET.fromstring(archive.read("EPUB/package.opf"))
            nav = ET.fromstring(archive.read("EPUB/nav.xhtml"))
        self.assertIn("<svg", digest_xhtml)

        manifest_hrefs = {
            item.attrib["href"]
            for item in package.findall("opf:manifest/opf:item", NS)
        }
        spine_idrefs = [
            item.attrib["idref"]
            for item in package.findall("opf:spine/opf:itemref", NS)
        ]
        nav_hrefs = {
            link.attrib["href"]
            for link in nav.findall(".//xhtml:a", NS)
        }

        self.assertIn("text/bibliosmith-digest.xhtml", manifest_hrefs)
        self.assertIn("bibliosmith-digest", spine_idrefs)
        self.assertIn("text/bibliosmith-digest.xhtml", nav_hrefs)


class BiblioSmithDigestDocumentationTest(unittest.TestCase):
    def test_digest_directory_contains_module_assets_not_only_scripts(self):
        expected_files = [
            REPO_ROOT / "packages" / "digest" / "prompts" / "01_digest_generation.zh-CN.md",
            REPO_ROOT / "packages" / "digest" / "prompts" / "02_digest_review.zh-CN.md",
            REPO_ROOT / "packages" / "digest" / "references" / "bibliosmith_digest_workflow.md",
            REPO_ROOT / "packages" / "digest" / "schemas" / "digest.config.schema.json",
            REPO_ROOT / "packages" / "digest" / "qa" / "digest_review_checklist.zh-CN.md",
        ]
        for path in expected_files:
            self.assertTrue(path.exists(), f"Missing Digest module asset: {path}")

    def test_main_readmes_expose_digest_with_same_language_links(self):
        readmes = [
            {
                "path": REPO_ROOT / "README.md",
                "link": "readme/digest/README.en.md",
            },
            {
                "path": REPO_ROOT / "README.zh-CN.md",
                "link": "readme/digest/README.zh-CN.md",
            },
        ]

        for item in readmes:
            text = item["path"].read_text("utf-8")
            self.assertIn("BiblioSmith Digest", text, str(item["path"]))
            self.assertIn(item["link"], text, str(item["path"]))
            self.assertIn("python -m digest.bibliosmith_digest --book-root", text, str(item["path"]))
            self.assertIn("digest.config.json", text, str(item["path"]))
            self.assertIn("books/local/", text, str(item["path"]))

    def test_how_to_use_prompt_guide_mentions_digest_decision_and_command(self):
        text = (
            REPO_ROOT
            / "docs"
            / "guides"
            / "how-to-use-local-reading.zh-CN.md"
        ).read_text("utf-8")
        self.assertIn("明确勾选 BiblioSmith Digest", text)
        self.assertIn("python -m digest.bibliosmith_digest --book-root", text)
        self.assertIn("digest.config.json", text)
        self.assertIn("输出仍然是标准 EPUB", text)

    def test_multilingual_readme_and_license_files_exist_and_link_languages(self):
        expected_files = [
            REPO_ROOT / "packages" / "digest" / "README.md",
            REPO_ROOT / "readme" / "digest" / "README.zh-CN.md",
            REPO_ROOT / "readme" / "digest" / "README.en.md",
            REPO_ROOT / "readme" / "digest" / "README.zh-TW.md",
            REPO_ROOT / "readme" / "digest" / "README.ja.md",
            REPO_ROOT / "license" / "DIGEST_LICENSE.md",
            REPO_ROOT / "license" / "DIGEST_LICENSE.en.md",
            REPO_ROOT / "license" / "DIGEST_LICENSE.zh-TW.md",
            REPO_ROOT / "license" / "DIGEST_LICENSE.ja.md",
        ]
        for path in expected_files:
            self.assertTrue(path.exists(), f"Missing {path}")

        language_labels = ["简体中文", "繁體中文", "English", "日本語"]
        quick_start_labels = ["Quick Start", "快速开始", "快速開始", "クイックスタート"]
        for path in expected_files[:5]:
            text = path.read_text("utf-8")
            for label in language_labels:
                self.assertIn(label, text, f"{path} missing {label}")
            self.assertTrue(
                any(label in text for label in quick_start_labels),
                f"{path} missing Quick Start section",
            )

    def test_upstream_project_name_is_confined_to_acknowledgement_and_license_docs(self):
        hits = []
        roots = [
            REPO_ROOT / "packages" / "digest",
            REPO_ROOT / "readme",
            REPO_ROOT / "license",
            REPO_ROOT / "doc" / "public",
            REPO_ROOT / "tests" / "digest",
        ]
        for root in roots:
            paths = root.rglob("*") if root.exists() else []
            for path in paths:
                if not path.is_file():
                    continue
                if any(
                    part == "__pycache__" or part.endswith(".egg-info")
                    for part in path.parts
                ):
                    continue
                text = path.read_text("utf-8", errors="ignore")
                restricted_name = "spine" + "digest"
                if restricted_name in text.lower():
                    hits.append(path.relative_to(REPO_ROOT).as_posix())

        allowed = {
            "packages/digest/README.md",
            "readme/digest/README.zh-CN.md",
            "readme/digest/README.en.md",
            "readme/digest/README.zh-TW.md",
            "readme/digest/README.ja.md",
            "license/DIGEST_LICENSE.md",
            "license/DIGEST_LICENSE.en.md",
            "license/DIGEST_LICENSE.zh-TW.md",
            "license/DIGEST_LICENSE.ja.md",
        }
        self.assertEqual(sorted(set(hits)), sorted(allowed))


def write_config(root, config):
    (root / "digest.config.json").write_text(
        json.dumps(config, ensure_ascii=False, indent=2),
        "utf-8",
    )


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def create_minimal_epub(root, title="测试书", chapters=None):
    output = root / "output"
    output.mkdir(parents=True)
    epub = output / "book.epub"
    chapters = chapters or [("第一章", "这是第一章的内容。它说明了故事的起点和主要问题。")]
    manifest_items = "\n".join(
        f'    <item id="chapter-{index}" href="text/chapter-{index}.xhtml" media-type="application/xhtml+xml" />'
        for index, _chapter in enumerate(chapters, start=1)
    )
    spine_items = "\n".join(
        f'    <itemref idref="chapter-{index}" />'
        for index, _chapter in enumerate(chapters, start=1)
    )
    nav_items = "\n".join(
        f'  <li><a href="text/chapter-{index}.xhtml">{chapter_title}</a></li>'
        for index, (chapter_title, _chapter_text) in enumerate(chapters, start=1)
    )
    files = {
        "mimetype": "application/epub+zip",
        "META-INF/container.xml": """<?xml version="1.0" encoding="utf-8"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="EPUB/package.opf" media-type="application/oebps-package+xml" />
  </rootfiles>
</container>
""",
        "EPUB/package.opf": """<?xml version="1.0" encoding="utf-8"?>
<package version="3.0" unique-identifier="bookid" xmlns="http://www.idpf.org/2007/opf">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:identifier id="bookid">urn:test</dc:identifier>
    <dc:title>""" + title + """</dc:title>
    <dc:language>zh-CN</dc:language>
  </metadata>
  <manifest>
    <item id="nav" href="nav.xhtml" media-type="application/xhtml+xml" properties="nav" />
""" + manifest_items + """
  </manifest>
  <spine>
""" + spine_items + """
  </spine>
</package>
""",
        "EPUB/nav.xhtml": """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops" xml:lang="zh-CN" lang="zh-CN">
<head><title>目录</title></head>
<body><nav epub:type="toc" id="toc"><h1>目录</h1><ol>
""" + nav_items + """
</ol></nav></body>
</html>
""",
    }
    for index, (chapter_title, chapter_text) in enumerate(chapters, start=1):
        files[f"EPUB/text/chapter-{index}.xhtml"] = """<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml" xml:lang="zh-CN" lang="zh-CN">
<head><title>""" + chapter_title + """</title></head>
<body><h1>""" + chapter_title + """</h1><p>""" + chapter_text + """</p></body>
</html>
"""
    with zipfile.ZipFile(epub, "w") as archive:
        archive.writestr("mimetype", files.pop("mimetype"), compress_type=zipfile.ZIP_STORED)
        for name, content in files.items():
            archive.writestr(name, content)
    return epub


if __name__ == "__main__":
    unittest.main()

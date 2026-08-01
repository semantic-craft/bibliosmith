import unittest

from translation_engine.chunking import TokenChunker, Utf8ByteTokenCounter


class TokenChunkerTests(unittest.TestCase):
    def test_chunks_respect_configured_token_limit_without_losing_text(self) -> None:
        text = (
            "Alpha beta gamma. Delta epsilon zeta.\n\n"
            "中文句子不会丢失。 ⟦PH_000000⟧ Final words.\n"
        )
        counter = Utf8ByteTokenCounter()
        chunks = TokenChunker(max_tokens=20, counter=counter).split(text)

        self.assertGreater(len(chunks), 1)
        self.assertEqual("".join(chunks), text)
        self.assertTrue(all(len(chunk.encode("utf-8")) <= 20 for chunk in chunks))

    def test_byte_upper_bound_does_not_ignore_repeated_underscores(self) -> None:
        text = "_" * 1000
        counter = Utf8ByteTokenCounter()
        chunks = TokenChunker(max_tokens=1, counter=counter).split(text)

        self.assertEqual("".join(chunks), text)
        self.assertEqual(len(chunks), 1000)
        self.assertTrue(all(len(chunk.encode("utf-8")) <= 1 for chunk in chunks))

    def test_html_table_chunks_prefer_complete_row_boundaries(self) -> None:
        text = (
            "Introductory text. "
            "<table><tr><td>Alpha</td><td>First description</td></tr>"
            "<tr><td>Beta</td><td>Second description</td></tr>"
            "<tr><td>Gamma</td><td>Third description</td></tr></table>"
        )
        counter = Utf8ByteTokenCounter()
        chunks = TokenChunker(max_tokens=95, counter=counter).split(text)

        self.assertEqual("".join(chunks), text)
        self.assertTrue(all(len(chunk.encode("utf-8")) <= 95 for chunk in chunks))
        self.assertTrue(chunks[0].endswith("</tr>"))
        self.assertTrue(chunks[1].endswith("</tr>"))
        self.assertFalse(any(chunk.endswith("description ") for chunk in chunks))

    def test_html_table_chunks_use_a_smaller_soft_limit(self) -> None:
        row = f"<tr><td>{'A' * 180}</td><td>{'B' * 180}</td></tr>"
        text = f"<table>{row * 8}</table>"
        counter = Utf8ByteTokenCounter()
        chunks = TokenChunker(max_tokens=2000, counter=counter).split(text)

        self.assertEqual("".join(chunks), text)
        self.assertGreater(len(chunks), 1)
        self.assertTrue(all(len(chunk.encode("utf-8")) <= 1024 for chunk in chunks))
        self.assertTrue(all(chunk.endswith("</tr>") for chunk in chunks[:-1]))


if __name__ == "__main__":
    unittest.main()

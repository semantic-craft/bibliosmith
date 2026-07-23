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


if __name__ == "__main__":
    unittest.main()

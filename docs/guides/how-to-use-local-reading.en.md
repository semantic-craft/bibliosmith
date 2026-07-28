# How to use local reading

1. Select or create a BiblioSmith repository folder in the Launcher.
2. Add an EPUB, PDF, Markdown, text file, or supported Zotero attachment that you already have locally.
3. Choose conversion, translation, and output options.
4. Review the sample and approve the bound translation plan before a full run.
5. Continue through translation, QA, promotion, EPUB build, and validation.
6. Open the requested artifacts under the book project's `output/reading/` directory.

The primary outputs are:

- Markdown: `output/reading/book.md`
- HTML: `output/reading/html/`
- EPUB: `output/reading/book.epub`
- Bilingual EPUB: `output/reading/book_bilingual.epub`
- Digest EPUB: `output/reading/book_digest.epub`
- Digest companion files: `output/reading/digest/`

For a compact reading edition, explicitly enable BiblioSmith Digest in the
Launcher. A manual run uses a book-local `digest.config.json` and:

```sh
python -m digest.bibliosmith_digest --book-root books/local/{target}/{number}_{title_author}
```

The result remains a standard EPUB.

Book projects live under `books/local/{target}/{number}_{title_author}/`. Source
files, translations, QA evidence, and generated reading artifacts stay local and
are ignored by Git.

The project does not search for book-length source text, remove DRM, bypass
access controls, or decide whether a work may be published.

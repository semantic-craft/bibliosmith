# Local Reading Validation Tools

`books/package.json` installs the shared EPUBCheck wrapper used by BiblioSmith
Launcher. Real book projects live under `books/local/` and remain ignored by
Git.

Install the validation dependency with:

```sh
npm --prefix books install
```

Project creation is handled by `tools/create_local_book_project.py`; this
directory contains no book-discovery, publishing, or release commands.

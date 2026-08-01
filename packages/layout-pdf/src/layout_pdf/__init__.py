"""Layout-preserving bilingual PDF track.

The reflow track turns a PDF into Markdown, translates it chapter by chapter
behind two approval gates and rebuilds an EPUB. This track does the opposite:
it hands the PDF to BabelDOC, which translates in place and writes a bilingual
PDF that still looks like the original. One pass, no gates, text PDFs only.
"""

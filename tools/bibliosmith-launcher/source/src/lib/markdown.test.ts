import { describe, expect, it } from "vitest";
import { copies } from "../i18n";
import { decodeCodePayload, renderMarkdownToHtml, sanitizeHref } from "./markdown";

const copy = copies.en;

function render(source: string) {
  return renderMarkdownToHtml(source, copy);
}

/**
 * The rendered string is fed to `dangerouslySetInnerHTML`, so what matters is
 * what a parser makes of it, not the markup's exact spelling. Parsing here is
 * also what makes the query-string regression legible: the defect was an href
 * escaped twice, which only shows up once an entity is resolved.
 */
function parse(html: string): HTMLElement {
  const host = document.createElement("div");
  host.innerHTML = html;
  return host;
}

describe("inline links", () => {
  // Regression: links used to be matched against already-escaped text, so
  // `&` in a query string reached sanitizeHref as `&amp;` and was escaped a
  // second time. Every link with a query string was broken.
  it("keeps a query string intact", () => {
    const anchor = parse(render("See [the docs](https://example.com/g?a=1&b=2).")).querySelector("a");
    expect(anchor?.getAttribute("href")).toBe("https://example.com/g?a=1&b=2");
    expect(anchor?.textContent).toBe("the docs");
  });

  it("escapes the href exactly once in the markup", () => {
    const html = render("[docs](https://example.com/g?a=1&b=2)");
    expect(html).toContain('href="https://example.com/g?a=1&amp;b=2"');
    expect(html).not.toContain("&amp;amp;");
  });

  it("still refuses a script-bearing protocol", () => {
    const anchor = parse(render("[click](javascript:alert(1))")).querySelector("a");
    expect(anchor?.getAttribute("href")).toBe("#");
  });

  it("escapes a label that contains markup", () => {
    const host = parse(render("[<img src=x onerror=alert(1)>](https://example.com/)"));
    expect(host.querySelector("img")).toBeNull();
    expect(host.querySelector("a")?.textContent).toBe("<img src=x onerror=alert(1)>");
  });

  it("escapes text on both sides of a link", () => {
    const host = parse(render("a <b> [x](https://example.com/) c <i> d"));
    expect(host.querySelector("b")).toBeNull();
    expect(host.querySelector("i")).toBeNull();
    expect(host.textContent).toBe("a <b> x c <i> d");
  });

  it("renders several links on one line", () => {
    const anchors = parse(render("[one](https://a.example/?x=1&y=2) and [two](https://b.example/)"))
      .querySelectorAll("a");
    expect(Array.from(anchors).map((anchor) => anchor.getAttribute("href"))).toEqual([
      "https://a.example/?x=1&y=2",
      "https://b.example/",
    ]);
  });

  it("renders emphasis and code alongside a link", () => {
    const host = parse(render("**bold** `code` [x](https://example.com/)"));
    expect(host.querySelector("strong")?.textContent).toBe("bold");
    expect(host.querySelector("code")?.textContent).toBe("code");
    expect(host.querySelector("a")?.getAttribute("href")).toBe("https://example.com/");
  });

  it("leaves a bare bracket pair alone", () => {
    const host = parse(render("an [unclosed link and (parens)"));
    expect(host.querySelector("a")).toBeNull();
    expect(host.textContent).toBe("an [unclosed link and (parens)");
  });
});

describe("block rendering", () => {
  it("renders headings up to four levels", () => {
    const host = parse(render("# One\n## Two\n#### Four\n##### Five"));
    expect(host.querySelector("h1")?.textContent).toBe("One");
    expect(host.querySelector("h2")?.textContent).toBe("Two");
    expect(host.querySelector("h4")?.textContent).toBe("Four");
    // Five hashes is past the contract, so it stays a paragraph.
    expect(host.querySelector("h5")).toBeNull();
    expect(host.querySelector("p")?.textContent).toBe("##### Five");
  });

  it("groups consecutive list items and closes the list after them", () => {
    const host = parse(render("- one\n- two\n\nafter"));
    expect(host.querySelectorAll("ul")).toHaveLength(1);
    expect(Array.from(host.querySelectorAll("li")).map((li) => li.textContent)).toEqual(["one", "two"]);
    expect(host.querySelector("p")?.textContent).toBe("after");
  });

  it("renders a link inside a list item", () => {
    const anchor = parse(render("- see [docs](https://example.com/?a=1&b=2)")).querySelector("li a");
    expect(anchor?.getAttribute("href")).toBe("https://example.com/?a=1&b=2");
  });

  it("keeps a fenced code block verbatim and carries it on the copy button", () => {
    const host = parse(render("```\nconst a = 1 < 2 && 3;\n```"));
    expect(host.querySelector("pre code")?.textContent).toBe("const a = 1 < 2 && 3;");
    const payload = host.querySelector("button")?.getAttribute("data-copy-code");
    expect(decodeCodePayload(payload ?? "")).toBe("const a = 1 < 2 && 3;");
  });

  it("does not treat markdown inside a code block as markup", () => {
    const host = parse(render("```\n# not a heading\n[x](https://example.com/)\n```"));
    expect(host.querySelector("h1")).toBeNull();
    expect(host.querySelector("a")).toBeNull();
  });

  it("closes an unterminated code block at the end of the document", () => {
    expect(parse(render("```\nstranded")).querySelector("pre code")?.textContent).toBe("stranded");
  });

  it("renders an embedded table through the safe-tag path", () => {
    const host = parse(render("<table><tr><td>cell</td></tr></table>"));
    expect(host.querySelector("table td")?.textContent).toBe("cell");
  });
});

describe("sanitizeHref", () => {
  it("allows the protocols the app links with", () => {
    expect(sanitizeHref("https://example.com/guide")).toBe("https://example.com/guide");
    expect(sanitizeHref("mailto:hi@example.com")).toBe("mailto:hi@example.com");
  });

  it("rejects anything else", () => {
    expect(sanitizeHref("javascript:alert(1)")).toBe("#");
    expect(sanitizeHref("data:text/html,<script>alert(1)</script>")).toBe("#");
  });
});

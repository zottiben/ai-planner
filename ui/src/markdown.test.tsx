// The markdown renderer is the only real logic on this side, and the only place a plan
// body becomes DOM. Both facts make it the thing worth testing.

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { Markdown } from "./markdown";
import { ago, prLabel, shortPath } from "./format";

function html(source: string): string {
  const { container } = render(<Markdown source={source} />);
  return container.innerHTML;
}

describe("Markdown", () => {
  it("never produces an element, whatever the body says", () => {
    const hostile = [
      "<script>alert(1)</script>",
      "<img src=x onerror=alert(1)>",
      "<iframe src='data:text/html,<script>alert(1)</script>'></iframe>",
      "<svg/onload=alert(1)>",
    ].join("\n\n");

    const { container } = render(<Markdown source={hostile} />);

    // Asserted against the DOM, not against the HTML string: the escaped text of a
    // hostile tag legitimately still contains the word "onerror", and a string match
    // would fail on safe output while passing on some unsafe output.
    expect(container.querySelector("script, img, iframe, svg, object, embed")).toBeNull();
    for (const element of container.querySelectorAll("*")) {
      for (const attribute of element.attributes) {
        expect(attribute.name.startsWith("on")).toBe(false);
      }
    }
    // All four survive as literal text - rule 6's bargain applied to rendering: what
    // we cannot classify is shown, never dropped.
    expect(screen.getAllByText(/alert\(1\)/)).toHaveLength(4);
  });

  it("refuses a javascript: link but keeps its text", () => {
    const out = html("[click me](javascript:alert(1))");
    expect(out).not.toContain("javascript:");
    expect(out).not.toContain("<a");
    expect(screen.getByText("click me")).toBeTruthy();
  });

  it("allows the schemes a plan actually uses", () => {
    for (const href of ["https://example.com/x", "http://example.com", "mailto:a@b.c"]) {
      const { container } = render(<Markdown source={`[label](${href})`} />);
      const anchor = container.querySelector("a");
      expect(anchor?.getAttribute("href")).toBe(href);
      // An untrusted link that can reach window.opener is a tabnabbing hole.
      expect(anchor?.getAttribute("rel")).toContain("noopener");
    }
  });

  it("renders the blocks plans are written in", () => {
    const out = html(
      [
        "## A heading",
        "",
        "Some **bold**, some *italic* and some `code`.",
        "",
        "- first",
        "- second",
        "",
        "1. one",
        "2. two",
        "",
        "> quoted",
        "",
        "```rust",
        "let x = 1;",
        "```",
        "",
        "| a | b |",
        "| --- | --- |",
        "| 1 | 2 |",
      ].join("\n"),
    );

    // Headings start two levels down: an <h1> inside a drawer misstates the outline.
    expect(out).toContain("<h4>A heading</h4>");
    expect(out).toContain("<strong>bold</strong>");
    expect(out).toContain("<em>italic</em>");
    expect(out).toContain("<code>code</code>");
    expect(out).toContain("<ul>");
    expect(out).toContain("<ol>");
    expect(out).toContain("<blockquote>");
    expect(out).toContain("let x = 1;");
    expect(out).toContain("<th>a</th>");
    expect(out).toContain("<td>1</td>");
    // The |---|---| row is formatting, not a row of data.
    expect(out).not.toContain("<td>---</td>");
  });

  it("always advances past lines that look like unsupported block openers", () => {
    const out = html(
      [
        "before",
        "             |",
        "after the pipe",
        "",
        "    # a shell comment, not a heading",
        "after the hash",
      ].join("\n"),
    );

    // Both forms exist in imported production plans. They used to match the
    // paragraph's stop condition without matching any block parser, so `blocks`
    // repeated the same line forever and froze the whole tab.
    expect(out).toContain("|");
    expect(out).toContain("# a shell comment, not a heading");
    expect(out).toContain("after the pipe");
    expect(out).toContain("after the hash");
  });

  it("does not swallow the rest of a document after an unterminated code fence", () => {
    const out = html("```\nunclosed\n");
    expect(out).toContain("unclosed");
  });

  it("keeps a wrapped list continuation with its item", () => {
    const out = html("- a point that runs\n  onto a second line\n- another");
    expect(out).toContain("a point that runs onto a second line");
    expect(out.match(/<li>/g)).toHaveLength(2);
  });
});

describe("format", () => {
  it("reads recent times as elapsed and old ones as dates", () => {
    const now = Date.now();
    expect(ago(new Date(now - 30_000).toISOString())).toBe("just now");
    expect(ago(new Date(now - 5 * 60_000).toISOString())).toBe("5m ago");
    expect(ago(new Date(now - 3 * 3_600_000).toISOString())).toBe("3h ago");
    expect(ago(new Date(now - 2 * 86_400_000).toISOString())).toBe("2d ago");
    expect(ago(null)).toBe("");
    expect(ago("not a date")).toBe("");
  });

  it("shortens a worktree path to the part that identifies it", () => {
    expect(shortPath("/Users/me/src/widget")).toBe("…/src/widget");
    expect(shortPath("/tmp")).toBe("/tmp");
    expect(shortPath(null)).toBe("");
  });

  it("reads a PR number out of a forge URL", () => {
    expect(prLabel("https://github.com/acme/widget/pull/42")).toBe("#42");
    expect(prLabel("https://gitlab.com/acme/w/-/merge_requests/7")).toBe("#7");
    expect(prLabel("https://example.com/something")).toBe("PR");
    expect(prLabel(null)).toBe("");
  });
});

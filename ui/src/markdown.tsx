// Markdown, rendered to React elements.
//
// Deliberately not `dangerouslySetInnerHTML` over a sanitised HTML string. These bodies
// are written by agents and by whatever markdown `aip import` swallowed, so they are
// untrusted input in the only sense that matters: nobody reviewed them. Building React
// elements makes that structural - the element types come from this file and every
// piece of text goes through React's escaping, so there is no filter to get wrong.
//
// It covers what plans actually contain. Anything it does not recognise comes out as
// its own literal text rather than disappearing, which is the same bargain rule 6
// makes for the importer.

import type { ReactNode } from "react";

/** Only schemes that cannot execute. `javascript:` in a plan body would otherwise be
 *  one click from running with the board's token in the page. */
function safeHref(href: string): string | undefined {
  const trimmed = href.trim();
  if (/^(https?:|mailto:)/i.test(trimmed)) return trimmed;
  if (trimmed.startsWith("/") || trimmed.startsWith("#")) return trimmed;
  return undefined;
}

const INLINE =
  /(`[^`]+`)|(\*\*[^*]+\*\*)|(__[^_]+__)|(\*[^*\n]+\*)|(~~[^~]+~~)|(\[[^\]]*\]\([^)\s]+\))|(https?:\/\/[^\s<>()]+)/g;

function inline(text: string, keyPrefix: string): ReactNode[] {
  const nodes: ReactNode[] = [];
  let last = 0;
  let match: RegExpExecArray | null;
  let n = 0;

  INLINE.lastIndex = 0;
  while ((match = INLINE.exec(text)) !== null) {
    if (match.index > last) nodes.push(text.slice(last, match.index));
    const token = match[0];
    const key = `${keyPrefix}-${n++}`;

    if (token.startsWith("`")) {
      nodes.push(<code key={key}>{token.slice(1, -1)}</code>);
    } else if (token.startsWith("**") || token.startsWith("__")) {
      nodes.push(<strong key={key}>{token.slice(2, -2)}</strong>);
    } else if (token.startsWith("~~")) {
      nodes.push(<del key={key}>{token.slice(2, -2)}</del>);
    } else if (token.startsWith("*")) {
      nodes.push(<em key={key}>{token.slice(1, -1)}</em>);
    } else if (token.startsWith("[")) {
      const split = token.indexOf("](");
      const label = token.slice(1, split);
      const href = safeHref(token.slice(split + 2, -1));
      // A link we will not follow still has to show its text, or the sentence loses
      // a word and nobody knows why.
      nodes.push(
        href ? (
          <a key={key} href={href} target="_blank" rel="noreferrer noopener">
            {label}
          </a>
        ) : (
          <span key={key}>{label}</span>
        ),
      );
    } else {
      nodes.push(
        <a key={key} href={token} target="_blank" rel="noreferrer noopener">
          {token}
        </a>,
      );
    }
    last = match.index + token.length;
  }

  if (last < text.length) nodes.push(text.slice(last));
  return nodes;
}

const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const NUMBERED = /^(\s*)\d+[.)]\s+(.*)$/;

export function Markdown({ source, className }: { source: string; className?: string }) {
  return <div className={`md${className ? ` ${className}` : ""}`}>{blocks(source)}</div>;
}

function blocks(source: string): ReactNode[] {
  const lines = source.replace(/\r\n/g, "\n").split("\n");
  const out: ReactNode[] = [];
  let i = 0;
  let key = 0;

  while (i < lines.length) {
    const line = lines[i] ?? "";

    if (line.trim() === "") {
      i += 1;
      continue;
    }

    // Fenced code. The closing fence is optional so an unterminated block still
    // renders the rest of the document instead of eating it.
    const fence = /^\s*```(\w*)\s*$/.exec(line);
    if (fence) {
      const body: string[] = [];
      i += 1;
      while (i < lines.length && !/^\s*```/.test(lines[i] ?? "")) {
        body.push(lines[i] ?? "");
        i += 1;
      }
      i += 1;
      out.push(
        <pre key={key++} data-lang={fence[1] || undefined}>
          <code>{body.join("\n")}</code>
        </pre>,
      );
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      const depth = (heading[1] ?? "#").length;
      // Headings inside a body sit under the pane's own heading, so they start two
      // levels down - an <h1> in a drawer is a lie about the document outline.
      const Tag = `h${Math.min(depth + 2, 6)}` as "h3";
      out.push(<Tag key={key++}>{inline(heading[2] ?? "", `h${key}`)}</Tag>);
      i += 1;
      continue;
    }

    if (/^\s*(---+|\*\*\*+|___+)\s*$/.test(line)) {
      out.push(<hr key={key++} />);
      i += 1;
      continue;
    }

    if (/^\s*>/.test(line)) {
      const body: string[] = [];
      while (i < lines.length && /^\s*>/.test(lines[i] ?? "")) {
        body.push((lines[i] ?? "").replace(/^\s*>\s?/, ""));
        i += 1;
      }
      out.push(<blockquote key={key++}>{blocks(body.join("\n"))}</blockquote>);
      continue;
    }

    if (/^\s*\|.*\|\s*$/.test(line)) {
      const rows: string[] = [];
      while (i < lines.length && /^\s*\|/.test(lines[i] ?? "")) {
        rows.push(lines[i] ?? "");
        i += 1;
      }
      out.push(<Table key={key++} rows={rows} />);
      continue;
    }

    if (BULLET.test(line) || NUMBERED.test(line)) {
      const ordered = !BULLET.test(line) && NUMBERED.test(line);
      const items: string[] = [];
      while (i < lines.length) {
        const current = lines[i] ?? "";
        const item = BULLET.exec(current) ?? NUMBERED.exec(current);
        if (item) {
          items.push(item[2] ?? "");
          i += 1;
        } else if (/^\s{2,}\S/.test(current) && items.length > 0) {
          // A wrapped continuation line belongs to the item above it.
          items[items.length - 1] += ` ${current.trim()}`;
          i += 1;
        } else {
          break;
        }
      }
      const List = ordered ? "ol" : "ul";
      out.push(
        <List key={key++}>
          {items.map((item, index) => (
            <li key={index}>{inline(item, `li${index}`)}</li>
          ))}
        </List>,
      );
      continue;
    }

    const paragraph: string[] = [];
    while (i < lines.length) {
      const current = lines[i] ?? "";
      if (
        current.trim() === "" ||
        /^\s*(#{1,6}\s|>|```|\||---+$)/.test(current) ||
        BULLET.test(current) ||
        NUMBERED.test(current)
      ) {
        break;
      }
      paragraph.push(current);
      i += 1;
    }
    out.push(<p key={key++}>{inline(paragraph.join(" "), `p${key}`)}</p>);
  }

  return out;
}

function Table({ rows }: { rows: string[] }) {
  const cells = (row: string) =>
    row
      .trim()
      .replace(/^\|/, "")
      .replace(/\|$/, "")
      .split("|")
      .map((cell) => cell.trim());

  const [head, ...rest] = rows;
  // The `|---|---|` separator is formatting, not data.
  const body = rest.filter((row) => !/^\s*\|[\s:|-]+\|\s*$/.test(row));

  return (
    <table>
      {head && (
        <thead>
          <tr>
            {cells(head).map((cell, index) => (
              <th key={index}>{inline(cell, `th${index}`)}</th>
            ))}
          </tr>
        </thead>
      )}
      <tbody>
        {body.map((row, rowIndex) => (
          <tr key={rowIndex}>
            {cells(row).map((cell, index) => (
              <td key={index}>{inline(cell, `td${rowIndex}-${index}`)}</td>
            ))}
          </tr>
        ))}
      </tbody>
    </table>
  );
}

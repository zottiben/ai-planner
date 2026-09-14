// Turning stored values into readable ones.
//
// Timestamps are ISO-8601 UTC TEXT because the schema is browsed in TablePlus (rule 17).
// That is the right storage choice and the wrong thing to show a person, so every
// timestamp goes through here.

import type { Status } from "./types";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;

export function ago(iso: string | null | undefined): string {
  if (!iso) return "";
  const then = Date.parse(iso);
  if (Number.isNaN(then)) return "";

  const delta = Date.now() - then;
  if (delta < MINUTE) return "just now";
  if (delta < HOUR) return `${Math.floor(delta / MINUTE)}m ago`;
  if (delta < DAY) return `${Math.floor(delta / HOUR)}h ago`;
  if (delta < 7 * DAY) return `${Math.floor(delta / DAY)}d ago`;
  return new Date(then).toLocaleDateString(undefined, {
    day: "numeric",
    month: "short",
    year: delta > 330 * DAY ? "numeric" : undefined,
  });
}

export function exact(iso: string | null | undefined): string {
  if (!iso) return "";
  const at = Date.parse(iso);
  return Number.isNaN(at) ? "" : new Date(at).toLocaleString();
}

/** The CSS variable holding a status colour. Kept in one place so a card, a dot and
 *  a column header can never disagree about what "blocked" looks like. */
export function statusColour(status: Status): string {
  return `var(--${status.replace("_", "-")})`;
}

/** `~/src/widget` reads better than the absolute path, and fits in a card. */
export function shortPath(path: string | null | undefined): string {
  if (!path) return "";
  const parts = path.split("/").filter(Boolean);
  return parts.length <= 2 ? path : `…/${parts.slice(-2).join("/")}`;
}

/** `https://github.com/acme/widget/pull/42` -> `#42`. */
export function prLabel(url: string | null | undefined): string {
  if (!url) return "";
  const match = /\/pull\/(\d+)/.exec(url) ?? /\/merge_requests\/(\d+)/.exec(url);
  return match?.[1] ? `#${match[1]}` : "PR";
}

// The ticket, opened.
//
// "Click further into a ticket to view what will be done" - so scope and demo lead,
// and everything mechanical (branch, PR, claim) sits under them where it can be
// scanned rather than read. The slice's own log is last, because it answers "what
// happened" only once you know what the thing is.

import { useEffect, useRef, useState } from "react";

import { api } from "./api";
import { ago, exact, prLabel, statusColour } from "./format";
import { useResource } from "./hooks";
import { Branch, Person, PullRequest } from "./icons";
import { Markdown } from "./markdown";
import { useToast } from "./Toast";
import type { Slice, Status, StatusMeta } from "./types";

interface Props {
  slice: Slice;
  planId: number;
  statuses: StatusMeta[];
  /** Bumped when another process writes, so an open ticket follows along too. */
  generation: number;
  onChanged: () => void;
  onMove: (slice: Slice, to: Status) => void;
  onClose: () => void;
}

export function Drawer({
  slice,
  planId,
  statuses,
  generation,
  onChanged,
  onMove,
  onClose,
}: Props) {
  const detail = useResource(() => api.slice(slice.id), [slice.id, generation]);
  const panel = useRef<HTMLDivElement>(null);
  const toast = useToast();
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Focus moves into the panel when it opens, so the keyboard follows the eye and
  // Escape reaches the handler above without a click first.
  useEffect(() => {
    panel.current?.focus();
  }, [slice.id]);

  // The board's copy of the slice is already on screen, so it renders immediately and
  // the fetched copy replaces it. The drawer never shows a spinner over data it has.
  const current = detail.data?.slice ?? slice;

  /** Run a mutation, then reload both the drawer and the board behind it.
   *
   *  The reload happens on failure too, and deliberately: a refusal is nearly always
   *  the news that somebody else changed this underneath us, so the screen is stale in
   *  exactly the moment it most matters. Leaving it showing "Claim" for a slice that is
   *  now held would invite the same click again. */
  const act = async (what: string, run: () => Promise<unknown>) => {
    setBusy(true);
    try {
      await run();
    } catch (error) {
      toast.blame(error, what);
    } finally {
      setBusy(false);
      detail.reload();
      onChanged();
    }
  };

  return (
    <>
      <div className="scrim" onClick={onClose} aria-hidden="true" />
      <aside
        className="drawer"
        ref={panel}
        tabIndex={-1}
        role="dialog"
        aria-label={`${current.key}: ${current.title}`}
      >
        <header className="drawer-head">
          <div className="drawer-head-row">
            <span className="card-key">{current.key}</span>
            <label className="status-select" style={{ color: statusColour(current.status) }}>
              <span className="dot" style={{ background: statusColour(current.status) }} />
              <select
                value={current.status}
                disabled={busy}
                aria-label="Status"
                onChange={(event) => onMove(current, event.target.value as Status)}
              >
                {statuses.map((s) => (
                  <option key={s.value} value={s.value}>
                    {s.label}
                  </option>
                ))}
              </select>
            </label>
            <button className="icon-button" onClick={onClose} aria-label="Close" title="Close (Esc)">
              ✕
            </button>
          </div>
          <h2>{current.title}</h2>

          <div className="drawer-actions">
            {current.claimed_by ? (
              <button
                className="button"
                disabled={busy}
                onClick={() => act(`Could not release ${current.key}`, () => api.release(current.id))}
              >
                Release
              </button>
            ) : (
              <button
                className="button primary"
                disabled={busy}
                onClick={() => act(`Could not claim ${current.key}`, () => api.claim(current.id))}
              >
                Claim
              </button>
            )}
            <button
              className="button"
              disabled={busy}
              onClick={() => {
                const url = window.prompt("Pull request URL", current.pr_url ?? "");
                if (url === null) return;
                void act("Could not save the pull request link", () =>
                  api.editSlice(current.id, { pr_url: url.trim() }),
                );
              }}
            >
              {current.pr_url ? "Change PR link" : "Link a PR"}
            </button>
          </div>
        </header>

        <div className="drawer-body">
          {current.blocked_reason && (
            <div className="callout blocked-callout">
              <b>Blocked</b>
              <span>{current.blocked_reason}</span>
            </div>
          )}

          {current.claimed_by && (
            <div className="callout">
              <b>
                <Person size={11} /> Held by {current.claimed_by}
              </b>
              <span>
                {current.worktree_path}
                {current.claimed_at && ` · since ${ago(current.claimed_at)}`}
              </span>
            </div>
          )}

          <Field label="Scope">
            {current.scope_md.trim() ? (
              <Markdown source={current.scope_md} />
            ) : (
              <p className="faint">Not written yet.</p>
            )}
          </Field>

          {current.demo_md && (
            <Field label="How to prove it works">
              <Markdown source={current.demo_md} />
            </Field>
          )}

          <Field label="Delivery">
            <dl className="facts">
              <Fact label="Branch" mono>
                {current.branch ? (
                  <>
                    <Branch size={11} /> {current.branch}
                  </>
                ) : null}
              </Fact>
              <Fact label="Base" mono>
                {current.base_branch}
              </Fact>
              <Fact label="Pull request">
                {current.pr_url ? (
                  <a href={current.pr_url} target="_blank" rel="noreferrer noopener">
                    <PullRequest size={11} /> {prLabel(current.pr_url)}
                  </a>
                ) : null}
              </Fact>
              <Fact label="Estimate">
                {current.estimate_files === null ? null : `${current.estimate_files} files`}
              </Fact>
              <Fact label="Started" title={exact(current.started_at)}>
                {ago(current.started_at)}
              </Fact>
              <Fact label="Completed" title={exact(current.completed_at)}>
                {ago(current.completed_at)}
              </Fact>
            </dl>
          </Field>

          <Field label={`Progress${detail.data ? ` (${detail.data.log.length})` : ""}`}>
            <NoteBox
              busy={busy}
              onSubmit={(body) =>
                act("Could not save the note", () =>
                  api.addNote(planId, body, current.key),
                )
              }
            />
            {detail.error && <p className="faint">{detail.error.message}</p>}
            {detail.data?.log.length === 0 && <p className="faint">Nothing recorded yet.</p>}
            <ol className="log">
              {detail.data?.log.map((entry) => (
                <li key={entry.id} className={`log-entry kind-${entry.kind}`}>
                  <div className="log-meta">
                    <span className="log-kind">{entry.kind}</span>
                    <span title={exact(entry.at)}>{ago(entry.at)}</span>
                    {entry.actor && <span>· {entry.actor}</span>}
                  </div>
                  <Markdown source={entry.body} className="tight" />
                </li>
              ))}
            </ol>
          </Field>
        </div>
      </aside>
    </>
  );
}

/** A note, written where the work is. The log is append-only and cannot conflict, so
 *  there is never a reason to batch these up - which is exactly why this box is here
 *  rather than behind a dialog. */
function NoteBox({
  busy,
  onSubmit,
}: {
  busy: boolean;
  onSubmit: (body: string) => Promise<void>;
}) {
  const [body, setBody] = useState("");

  const send = async () => {
    const text = body.trim();
    if (!text || busy) return;
    await onSubmit(text);
    setBody("");
  };

  return (
    <div className="note-box">
      <textarea
        value={body}
        rows={2}
        placeholder="Record what happened…"
        aria-label="Progress note"
        disabled={busy}
        onChange={(event) => setBody(event.target.value)}
        onKeyDown={(event) => {
          // Enter inserts a newline, because notes are markdown and often more than
          // one line. Cmd/Ctrl-Enter sends, which is what every other note box does.
          if (event.key === "Enter" && (event.metaKey || event.ctrlKey)) {
            event.preventDefault();
            void send();
          }
        }}
      />
      {body.trim() !== "" && (
        <div className="note-box-foot">
          <span className="faint small">⌘↵ to save</span>
          <button className="button primary" disabled={busy} onClick={() => void send()}>
            Save note
          </button>
        </div>
      )}
    </div>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <section className="field">
      <h3>{label}</h3>
      {children}
    </section>
  );
}

/** A fact with nothing in it is dropped rather than shown as an empty row - six
 *  labelled blanks tell you less than the two values that exist. */
function Fact({
  label,
  children,
  mono,
  title,
}: {
  label: string;
  children: React.ReactNode;
  mono?: boolean;
  title?: string;
}) {
  if (children === null || children === undefined || children === "") return null;
  return (
    <>
      <dt>{label}</dt>
      <dd className={mono ? "mono" : undefined} title={title}>
        {children}
      </dd>
    </>
  );
}

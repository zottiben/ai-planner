// The ticket, opened.
//
// "Click further into a ticket to view what will be done" - so scope and demo lead,
// and everything mechanical (branch, PR, claim) sits under them where it can be
// scanned rather than read. The slice's own log is last, because it answers "what
// happened" only once you know what the thing is.

import { useEffect, useRef } from "react";

import { api } from "./api";
import { ago, exact, prLabel, statusColour } from "./format";
import { useResource } from "./hooks";
import { Branch, Person, PullRequest } from "./icons";
import { Markdown } from "./markdown";
import type { Slice, StatusMeta } from "./types";

interface Props {
  slice: Slice;
  statuses: StatusMeta[];
  onClose: () => void;
}

export function Drawer({ slice, statuses, onClose }: Props) {
  const detail = useResource(() => api.slice(slice.id), [slice.id]);
  const panel = useRef<HTMLDivElement>(null);

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

  const status = statuses.find((s) => s.value === slice.status);
  // The board's copy of the slice is already on screen, so it renders immediately and
  // the fetched copy replaces it. The drawer never shows a spinner over data it has.
  const current = detail.data?.slice ?? slice;

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
            <span className="status-pill" style={{ color: statusColour(current.status) }}>
              <span className="dot" style={{ background: statusColour(current.status) }} />
              {status?.label ?? current.status}
            </span>
            <button className="icon-button" onClick={onClose} aria-label="Close" title="Close (Esc)">
              ✕
            </button>
          </div>
          <h2>{current.title}</h2>
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

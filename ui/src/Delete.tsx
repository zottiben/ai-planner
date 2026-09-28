// The one screen on this board that asks twice.
//
// Every other write here can be undone by making the opposite one, which is why they
// all happen on a single click. Deleting cannot, so it gets the three things the CLI
// and the MCP tool already insist on: the size of the thing, counted from the database
// rather than guessed at; a refusal while somebody else is holding the work; and, for
// a plan, its own slug typed back, so a click on the wrong row in the sidebar cannot
// carry all the way through.

import { useEffect, useRef, useState } from "react";

import { api } from "./api";
import { useResource } from "./hooks";
import { useToast } from "./Toast";
import type { HeldSlice, PlanSummary, Slice } from "./types";

/** A count worth showing. Zeroes are dropped - six labelled noughts say less than the
 *  two numbers that are not. */
function counted(value: number, one: string, many: string): string | null {
  if (value <= 0) return null;
  return `${value} ${value === 1 ? one : many}`;
}

interface ShellProps {
  heading: string;
  subject: string;
  /** What the server says is about to go. Absent while it is still being asked. */
  summary: string[] | undefined;
  /** What survives, and where. Reassurance that is true, not a disclaimer. */
  keeps?: React.ReactNode;
  held: HeldSlice[];
  /** Set for a plan: the exact text that has to be typed before Delete works. */
  typeToConfirm?: string;
  error?: Error;
  busy: boolean;
  onCancel: () => void;
  onConfirm: (force: boolean) => void;
}

function Shell({
  heading,
  subject,
  summary,
  keeps,
  held,
  typeToConfirm,
  error,
  busy,
  onCancel,
  onConfirm,
}: ShellProps) {
  const panel = useRef<HTMLDivElement>(null);
  const [typed, setTyped] = useState("");
  const [force, setForce] = useState(false);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  useEffect(() => {
    panel.current?.focus();
  }, []);

  const named = typeToConfirm === undefined || typed.trim() === typeToConfirm;
  const cleared = held.length === 0 || force;
  const ready = summary !== undefined && named && cleared && !busy;

  return (
    <>
      {/* Not dismissed by a click outside: the reply to "are you sure" is a button,
          and a stray click is exactly what this dialog exists to catch. */}
      <div className="scrim is-dialog" aria-hidden="true" />
      <div
        className="dialog"
        ref={panel}
        tabIndex={-1}
        role="dialog"
        aria-modal="true"
        aria-label={heading}
      >
        <h2>{heading}</h2>
        <p className="dialog-subject">{subject}</p>

        {error && <p className="dialog-error">{error.message}</p>}

        {summary === undefined && !error ? (
          <p className="faint">Counting what would go…</p>
        ) : (
          summary &&
          summary.length > 0 && (
            <ul className="dialog-list">
              {summary.map((line) => (
                <li key={line}>{line}</li>
              ))}
            </ul>
          )
        )}

        {keeps && <p className="faint small">{keeps}</p>}

        {held.length > 0 && (
          <div className="callout blocked-callout">
            <b>Somebody is working on this</b>
            {held.map((h) => (
              <span key={h.key}>
                {h.key} is claimed by {h.claimed_by} in {h.worktree_path}
              </span>
            ))}
            <label className="dialog-force">
              <input
                type="checkbox"
                checked={force}
                disabled={busy}
                onChange={(event) => setForce(event.target.checked)}
              />
              Delete it anyway, and throw that work away
            </label>
          </div>
        )}

        {typeToConfirm !== undefined && (
          <label className="dialog-type">
            Type <b>{typeToConfirm}</b> to confirm
            <input
              className="search"
              value={typed}
              autoFocus
              spellCheck={false}
              autoComplete="off"
              aria-label={`Type ${typeToConfirm} to confirm`}
              disabled={busy}
              onChange={(event) => setTyped(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter" && ready) onConfirm(force);
              }}
            />
          </label>
        )}

        <div className="dialog-actions">
          <button className="button" disabled={busy} onClick={onCancel}>
            Cancel
          </button>
          <button
            className="button danger"
            disabled={!ready}
            onClick={() => onConfirm(force)}
          >
            {busy ? "Deleting…" : "Delete"}
          </button>
        </div>
      </div>
    </>
  );
}

export function DeletePlanDialog({
  plan,
  onDeleted,
  onCancel,
}: {
  plan: PlanSummary;
  onDeleted: () => void;
  onCancel: () => void;
}) {
  const preview = useResource(() => api.planRemoval(plan.id), [plan.id]);
  const [busy, setBusy] = useState(false);
  const toast = useToast();

  const r = preview.data;
  const summary = r && [
    counted(r.slices, "slice", "slices"),
    counted(r.sections, "section", "sections"),
    counted(r.decisions, "decision", "decisions"),
    counted(r.questions, "question", "questions"),
    counted(r.gotchas, "gotcha", "gotchas"),
    counted(r.log_entries, "note", "notes"),
    counted(r.handoffs, "handoff", "handoffs"),
    counted(r.sources, "source", "sources"),
  ].filter((line): line is string => line !== null);

  const remove = async (force: boolean) => {
    setBusy(true);
    try {
      await api.deletePlan(plan.id, plan.slug, force);
      toast.say(`Deleted ${plan.slug}`, plan.title);
      onDeleted();
    } catch (error) {
      toast.blame(error, `Could not delete ${plan.slug}`);
      setBusy(false);
    }
  };

  return (
    <Shell
      heading="Delete this plan?"
      subject={plan.title}
      summary={summary}
      keeps={
        r?.imported_from.length ? (
          <>
            This is the only copy. It was imported from {r.imported_from.join(", ")}, and
            the stored copy of that goes with it.
          </>
        ) : (
          <>
            This is the only copy. <code>aip export</code> writes a plan back out as
            markdown, and <code>aip db backup</code> copies the whole database.
          </>
        )
      }
      held={r?.held ?? []}
      typeToConfirm={plan.slug}
      error={preview.error}
      busy={busy}
      onCancel={onCancel}
      onConfirm={remove}
    />
  );
}

export function DeleteSliceDialog({
  slice,
  onDeleted,
  onCancel,
}: {
  slice: Slice;
  onDeleted: () => void;
  onCancel: () => void;
}) {
  const preview = useResource(() => api.sliceRemoval(slice.id), [slice.id]);
  const [busy, setBusy] = useState(false);
  const toast = useToast();

  const r = preview.data;
  const summary = r
    ? [counted(r.dependents, "slice depends on it", "slices depend on it")].filter(
        (line): line is string => line !== null,
      )
    : undefined;

  const detached = r
    ? [
        counted(r.detached_log_entries, "note", "notes"),
        counted(r.detached_questions, "question", "questions"),
      ].filter((line): line is string => line !== null)
    : [];

  const remove = async (force: boolean) => {
    setBusy(true);
    try {
      await api.deleteSlice(slice.id, force);
      toast.say(`Deleted ${slice.key}`, slice.title);
      onDeleted();
    } catch (error) {
      toast.blame(error, `Could not delete ${slice.key}`);
      setBusy(false);
    }
  };

  return (
    <Shell
      heading={`Delete ${slice.key}?`}
      subject={slice.title}
      summary={summary}
      keeps={
        detached.length > 0
          ? `${detached.join(" and ")} written against it stay on the plan - the record of what happened outlives the slice it was about.`
          : "The plan keeps a note saying this was deleted."
      }
      held={r?.held ? [r.held] : []}
      error={preview.error}
      busy={busy}
      onCancel={onCancel}
      onConfirm={remove}
    />
  );
}

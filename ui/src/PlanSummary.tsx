import { useState } from "react";

/** Compact over the board, but never a dead ellipsis. A button gives mouse, Enter and
 * Space behavior without hand-rolled key handlers, and `key={plan.id}` at the call
 * site resets it when the user chooses another plan. */
export function PlanSummary({ text }: { text: string }) {
  const [expanded, setExpanded] = useState(false);

  return (
    <button
      type="button"
      className={`plan-summary${expanded ? " is-expanded" : ""}`}
      aria-expanded={expanded}
      title={expanded ? "Collapse plan description" : "Expand full plan description"}
      onClick={() => setExpanded((value) => !value)}
    >
      {text}
    </button>
  );
}

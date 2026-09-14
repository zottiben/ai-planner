// The board: seven columns, one per status.
//
// Every status gets a column even when it is empty (D6, and PR4 needs somewhere to drop
// a card). `done` and `deferred` start collapsed, because a finished plan is otherwise
// a wall of green that pushes the live work off the right of the screen.

import { useStored } from "./hooks";
import { Card } from "./Card";
import { Collapse, Expand } from "./icons";
import type { Board as BoardData, Slice, StatusMeta } from "./types";

interface Props {
  board: BoardData;
  statuses: StatusMeta[];
  currentSliceKey: string | undefined;
  onOpen: (slice: Slice) => void;
}

// Seven full-width columns do not fit a laptop, and these three are usually empty:
// `draft` is a slice still being written, and the two terminal ones are history. They
// collapse to a labelled strip you can still drag onto, rather than disappearing (D6).
const COLLAPSED_BY_DEFAULT = ["draft", "done", "deferred"];

export function Board({ board, statuses, currentSliceKey, onOpen }: Props) {
  const [collapsed, setCollapsed] = useStored<string[]>(
    "ai-planner.collapsed-columns",
    COLLAPSED_BY_DEFAULT,
  );

  const byStatus = new Map(board.columns.map((column) => [column.status, column.slices]));

  const toggle = (status: string) =>
    setCollapsed(
      collapsed.includes(status)
        ? collapsed.filter((s) => s !== status)
        : [...collapsed, status],
    );

  return (
    <div className="board">
      {statuses.map((meta) => {
        const slices = byStatus.get(meta.value) ?? [];
        // Collapsed regardless of what it holds, because a plan that is mostly done is
        // exactly the case D6 is about - and the strip still carries its name and its
        // count, so nothing is hidden, only folded.
        const isCollapsed = collapsed.includes(meta.value);

        return (
          <section
            key={meta.value}
            className={`column${isCollapsed ? " is-collapsed" : ""}`}
            aria-label={`${meta.label}, ${slices.length} slices`}
          >
            <header className="column-head">
              <span className="dot" style={{ background: `var(--${meta.value.replace("_", "-")})` }} />
              <span className="column-title">{meta.label}</span>
              <span className="column-count">{slices.length}</span>
              <button
                className="column-collapse"
                onClick={() => toggle(meta.value)}
                aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${meta.label}`}
                title={isCollapsed ? "Expand" : "Collapse"}
              >
                {isCollapsed ? <Expand size={11} /> : <Collapse size={11} />}
              </button>
            </header>

            {!isCollapsed && (
              <div className="column-body">
                {slices.map((slice) => (
                  <Card
                    key={slice.id}
                    slice={slice}
                    current={slice.key === currentSliceKey}
                    onOpen={onOpen}
                  />
                ))}
                {slices.length === 0 && <p className="column-empty">Nothing here.</p>}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

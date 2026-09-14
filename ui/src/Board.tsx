// The board: seven columns, one per status, and a write surface.
//
// Every status gets a column even when it is empty (D6) - a column that vanishes when
// it empties is a column you cannot drag a card into. draft, done and deferred fold to
// a labelled strip, which still accepts a drop.
//
// Dragging is `set_slice_status`, so the move is optimistic and the rollback is real:
// the card returns to the column it came from when the server refuses, rather than
// leaving the board asserting something the database does not agree with.

import { useState } from "react";

import { Card } from "./Card";
import { useStored } from "./hooks";
import { Collapse, Expand } from "./icons";
import type { Board as BoardData, Slice, Status, StatusMeta } from "./types";

interface Props {
  board: BoardData;
  statuses: StatusMeta[];
  currentSliceKey: string | undefined;
  onOpen: (slice: Slice) => void;
  onMove: (slice: Slice, to: Status) => void;
}

// Seven full-width columns do not fit a laptop, and these three are usually empty:
// `draft` is a slice still being written, and the two terminal ones are history.
const COLLAPSED_BY_DEFAULT = ["draft", "done", "deferred"];

export function Board({ board, statuses, currentSliceKey, onOpen, onMove }: Props) {
  const [collapsed, setCollapsed] = useStored<string[]>(
    "ai-planner.collapsed-columns",
    COLLAPSED_BY_DEFAULT,
  );
  const [dragging, setDragging] = useState<Slice | null>(null);
  const [over, setOver] = useState<Status | null>(null);

  const byStatus = new Map(board.columns.map((column) => [column.status, column.slices]));

  const toggle = (status: string) =>
    setCollapsed(
      collapsed.includes(status)
        ? collapsed.filter((s) => s !== status)
        : [...collapsed, status],
    );

  const drop = (to: Status) => {
    setOver(null);
    const slice = dragging;
    setDragging(null);
    if (slice && slice.status !== to) onMove(slice, to);
  };

  return (
    <div className="board">
      {statuses.map((meta) => {
        const slices = byStatus.get(meta.value) ?? [];
        const isCollapsed = collapsed.includes(meta.value);
        const isTarget = over === meta.value && dragging?.status !== meta.value;

        return (
          <section
            key={meta.value}
            className={`column${isCollapsed ? " is-collapsed" : ""}${isTarget ? " is-target" : ""}`}
            aria-label={`${meta.label}, ${slices.length} slices`}
            onDragOver={(event) => {
              if (!dragging) return;
              // Without preventDefault the browser refuses the drop, and the card
              // springs back with no error anywhere - a silent no-op.
              event.preventDefault();
              event.dataTransfer.dropEffect = "move";
              setOver(meta.value);
            }}
            onDragLeave={(event) => {
              // Leaving for a child element still fires dragleave on the parent, so
              // the highlight would flicker over every card in the column.
              if (!event.currentTarget.contains(event.relatedTarget as Node | null)) {
                setOver((current) => (current === meta.value ? null : current));
              }
            }}
            onDrop={(event) => {
              event.preventDefault();
              drop(meta.value);
            }}
          >
            <header className="column-head">
              <span
                className="dot"
                style={{ background: `var(--${meta.value.replace("_", "-")})` }}
              />
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
                    dragging={dragging?.id === slice.id}
                    onOpen={onOpen}
                    onDragStart={() => setDragging(slice)}
                    onDragEnd={() => {
                      setDragging(null);
                      setOver(null);
                    }}
                  />
                ))}
                {slices.length === 0 && (
                  <p className="column-empty">{dragging ? "Drop here" : "Nothing here."}</p>
                )}
              </div>
            )}
          </section>
        );
      })}
    </div>
  );
}

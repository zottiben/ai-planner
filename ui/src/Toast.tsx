// Telling the user what the server refused, and why.
//
// A board that says "failed" after a rejected drag is worse than one that does not
// move the card at all, because it hides the single most useful thing this tool can
// report: somebody else is already on this, here is who, and here is where. So a claim
// clash gets its own shape rather than being flattened into a string.

import { createContext, useCallback, useContext, useMemo, useState } from "react";

import { ApiError } from "./api";

export interface Note {
  id: number;
  tone: "info" | "error";
  message: string;
  detail?: string;
}

interface Sink {
  say: (message: string, detail?: string) => void;
  /** Turns whatever was thrown into something worth reading. */
  blame: (error: unknown, context: string) => void;
}

const ToastContext = createContext<Sink>({ say: () => {}, blame: () => {} });

export function useToast() {
  return useContext(ToastContext);
}

export function Toaster({ children }: { children: React.ReactNode }) {
  const [notes, setNotes] = useState<Note[]>([]);

  const push = useCallback((note: Omit<Note, "id">) => {
    const id = Date.now() + Math.random();
    setNotes((current) => [...current, { ...note, id }]);
    // Errors stay long enough to read a path; confirmations do not need to.
    const life = note.tone === "error" ? 9000 : 3200;
    window.setTimeout(() => setNotes((c) => c.filter((n) => n.id !== id)), life);
  }, []);

  const sink = useMemo<Sink>(
    () => ({
      say: (message, detail) => push({ tone: "info", message, detail }),
      blame: (error, context) => {
        if (error instanceof ApiError && error.code === "already_claimed") {
          push({
            tone: "error",
            message: `${error.slice ?? "That slice"} is held by ${error.holder}`,
            detail: error.worktree ? `in ${error.worktree}` : undefined,
          });
          return;
        }
        push({
          tone: "error",
          message: context,
          detail: error instanceof Error ? error.message : String(error),
        });
      },
    }),
    [push],
  );

  return (
    <ToastContext.Provider value={sink}>
      {children}
      <div className="toaster" role="status" aria-live="polite">
        {notes.map((note) => (
          <div key={note.id} className={`toast tone-${note.tone}`}>
            <b>{note.message}</b>
            {note.detail && <span>{note.detail}</span>}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

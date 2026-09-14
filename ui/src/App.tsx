import { useCallback, useEffect, useMemo, useState } from "react";

import { api } from "./api";
import { Board } from "./Board";
import { Drawer } from "./Drawer";
import { ago } from "./format";
import { useResource } from "./hooks";
import { Logo } from "./icons";
import { useLive } from "./live";
import { aboutPath, navigate, parse, planPath, slicePath, usePath } from "./router";
import { Rundown } from "./Rundown";
import { Sidebar } from "./Sidebar";
import { useToast } from "./Toast";
import type { Board as BoardData, Slice, Status } from "./types";

export function App() {
  const toast = useToast();
  const path = usePath();
  const route = useMemo(() => parse(path), [path]);
  const [search, setSearch] = useState("");

  // Every read depends on the liveness generation, so an agent writing from another
  // process refreshes what is on screen without anyone reaching for reload.
  const { generation, connection } = useLive();

  const meta = useResource(() => api.meta(), []);
  const repos = useResource(() => api.repos(), [generation]);
  const plans = useResource(() => api.plans(), [generation]);

  const plan = useMemo(
    () => plans.data?.find((p) => p.slug === route.planSlug),
    [plans.data, route.planSlug],
  );

  const board = useResource(
    () => (plan ? api.board(plan.id) : Promise.resolve(undefined)),
    [plan?.id, generation],
  );

  // A move the server has not confirmed yet. The board renders through it so a drag
  // lands instantly, and it is dropped - not merged - as soon as the truth arrives.
  const [pending, setPending] = useState<{ id: number; to: Status } | null>(null);

  const refreshAll = useCallback(() => {
    board.reload();
    plans.reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [board.reload, plans.reload]);

  const move = useCallback(
    async (slice: Slice, to: Status) => {
      // `set_slice_status` refuses to block something without a reason, so ask for it
      // here rather than sending a request that is known to fail.
      let reason: string | undefined;
      if (to === "blocked") {
        const answer = window.prompt(`Why is ${slice.key} blocked?`);
        if (answer === null || answer.trim() === "") return;
        reason = answer.trim();
      }

      setPending({ id: slice.id, to });
      try {
        await api.setSliceStatus(slice.id, to, reason);
        refreshAll();
      } catch (error) {
        // The rollback is simply forgetting the optimistic move: the card is drawn
        // from the board's own data again, which still holds the old status.
        toast.blame(error, `Could not move ${slice.key} to ${to.replace("_", " ")}`);
      } finally {
        setPending(null);
      }
    },
    [refreshAll, toast],
  );

  useEffect(() => {
    document.title = plan ? `${plan.title} · ai-planner` : "ai-planner";
  }, [plan]);

  if (meta.error) {
    return (
      <Fatal
        title="Cannot reach the board"
        detail={
          meta.error.message.includes("token")
            ? "The token in the URL is missing or stale. Start it again with `aip ui`."
            : meta.error.message
        }
      />
    );
  }

  const openSlice = (slice: Slice) => {
    if (!plan) return;
    navigate(slicePath(plan.slug, slice.key));
  };

  // The drawer is driven by the URL, so a ticket can be pasted into a message and a
  // reload lands back on it. The slice comes from the board that is already loaded.
  const shown = useMemo(
    () => (board.data ? withPendingMove(board.data, pending) : undefined),
    [board.data, pending],
  );

  const openTicket = route.sliceKey
    ? shown?.columns.flatMap((column) => column.slices).find((s) => s.key === route.sliceKey)
    : undefined;

  return (
    <div className="shell">
      <Sidebar
        repos={repos.data ?? []}
        plans={plans.data ?? []}
        currentSlug={route.planSlug}
        search={search}
        onSearch={setSearch}
      />

      <main className="main">
        {!route.planSlug && <Welcome loading={plans.loading} count={plans.data?.length ?? 0} />}

        {route.planSlug && !plan && !plans.loading && (
          <Fatal
            title="No such plan"
            detail={`Nothing in the database has the slug "${route.planSlug}".`}
          />
        )}

        {plan && (
          <>
            <header className="plan-head">
              <div className="plan-head-top">
                <div>
                  <div className="crumb">{plan.repo_name}</div>
                  <h1>{plan.title}</h1>
                </div>
                <div className="plan-head-right">
                  {plan.ticket_url && plan.ticket_key && (
                    <a href={plan.ticket_url} target="_blank" rel="noreferrer">
                      {plan.ticket_key}
                    </a>
                  )}
                  <div className="progress-figure">
                    <b>{plan.percent === null ? "—" : `${plan.percent}%`}</b>
                    <span>
                      {plan.done}/{plan.slices} slices
                    </span>
                  </div>
                </div>
              </div>

              {plan.summary && <p className="plan-summary">{plan.summary}</p>}

              <div className="tabs">
                <button
                  className={`tab${route.tab !== "plan" ? " is-current" : ""}`}
                  onClick={() => navigate(planPath(plan.slug))}
                >
                  Board
                </button>
                <button
                  className={`tab${route.tab === "plan" ? " is-current" : ""}`}
                  onClick={() => navigate(aboutPath(plan.slug))}
                >
                  Plan
                </button>
                <span className="tab connection" style={{ marginLeft: "auto" }}>
                  <span className={`live-dot is-${connection}`} />
                  {connection === "live" ? `updated ${ago(plan.updated_at)}` : connection}
                </span>
              </div>
            </header>

            {route.tab === "plan" ? (
              <Rundown
                planId={plan.id}
                generation={generation}
                onOpenSlice={(key) => navigate(slicePath(plan.slug, key))}
              />
            ) : (
              <>
                {board.error && (
                  <Fatal title="Cannot load the board" detail={board.error.message} />
                )}
                {shown && meta.data && (
                  <Board
                    board={shown}
                    statuses={meta.data.statuses}
                    currentSliceKey={route.sliceKey}
                    onOpen={openSlice}
                    onMove={move}
                  />
                )}
                {!shown && !board.error && <BoardSkeleton />}
              </>
            )}

            {openTicket && meta.data && (
              <Drawer
                slice={openTicket}
                planId={plan.id}
                statuses={meta.data.statuses}
                generation={generation}
                onChanged={refreshAll}
                onMove={move}
                onClose={() => navigate(planPath(plan.slug))}
              />
            )}
          </>
        )}
      </main>
    </div>
  );
}

function Welcome({ loading, count }: { loading: boolean; count: number }) {
  if (loading) return <BoardSkeleton />;
  return (
    <div className="centred">
      <Logo size={28} />
      <h2>{count === 0 ? "No plans yet" : "Pick a plan"}</h2>
      <p>
        {count === 0 ? (
          <>
            Start one with <code>aip new "&lt;title&gt;"</code>, or bring existing
            markdown in with <code>aip import --scan .</code>
          </>
        ) : (
          <>
            {count} {count === 1 ? "plan" : "plans"} in this database. Choose one on the
            left to see its board.
          </>
        )}
      </p>
    </div>
  );
}

function Fatal({ title, detail }: { title: string; detail: string }) {
  return (
    <div className="centred">
      <h2>{title}</h2>
      <p>{detail}</p>
    </div>
  );
}

/** Redraw the board with an unconfirmed move applied. Rolling back is then just
 *  discarding this, rather than a second mutation that could itself fail. */
function withPendingMove(
  board: BoardData,
  pending: { id: number; to: Status } | null,
): BoardData {
  if (!pending) return board;
  const moved = board.columns.flatMap((c) => c.slices).find((s) => s.id === pending.id);
  if (!moved || moved.status === pending.to) return board;

  return {
    ...board,
    columns: board.columns.map((column) => {
      if (column.status === moved.status) {
        return { ...column, slices: column.slices.filter((s) => s.id !== pending.id) };
      }
      if (column.status === pending.to) {
        return { ...column, slices: [{ ...moved, status: pending.to }, ...column.slices] };
      }
      return column;
    }),
  };
}

/** Columns at their real width, so nothing moves when the data lands. */
function BoardSkeleton() {
  return (
    <div className="board" aria-busy="true" aria-label="Loading">
      {[0, 1, 2, 3].map((column) => (
        <section className="column" key={column}>
          <header className="column-head">
            <span className="skeleton" style={{ height: 10, width: 68 }} />
          </header>
          <div className="column-body">
            {Array.from({ length: 3 - column % 2 }, (_, row) => (
              <span
                key={row}
                className="skeleton"
                style={{ height: 54, animationDelay: `${(column * 3 + row) * 60}ms` }}
              />
            ))}
          </div>
        </section>
      ))}
    </div>
  );
}

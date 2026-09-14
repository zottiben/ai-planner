import { useEffect, useMemo, useState } from "react";

import { api } from "./api";
import { Board } from "./Board";
import { Drawer } from "./Drawer";
import { ago } from "./format";
import { useResource } from "./hooks";
import { Logo } from "./icons";
import { aboutPath, navigate, parse, planPath, slicePath, usePath } from "./router";
import { Rundown } from "./Rundown";
import { Sidebar } from "./Sidebar";
import type { Slice } from "./types";

export function App() {
  const path = usePath();
  const route = useMemo(() => parse(path), [path]);
  const [search, setSearch] = useState("");

  const meta = useResource(() => api.meta(), []);
  const repos = useResource(() => api.repos(), []);
  const plans = useResource(() => api.plans(), []);

  const plan = useMemo(
    () => plans.data?.find((p) => p.slug === route.planSlug),
    [plans.data, route.planSlug],
  );

  const board = useResource(
    () => (plan ? api.board(plan.id) : Promise.resolve(undefined)),
    [plan?.id],
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
  const openTicket = route.sliceKey
    ? board.data?.columns.flatMap((column) => column.slices).find((s) => s.key === route.sliceKey)
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
                <span className="tab" style={{ marginLeft: "auto", cursor: "default" }}>
                  updated {ago(plan.updated_at)}
                </span>
              </div>
            </header>

            {route.tab === "plan" ? (
              <Rundown
                planId={plan.id}
                onOpenSlice={(key) => navigate(slicePath(plan.slug, key))}
              />
            ) : (
              <>
                {board.error && (
                  <Fatal title="Cannot load the board" detail={board.error.message} />
                )}
                {board.data && meta.data && (
                  <Board
                    board={board.data}
                    statuses={meta.data.statuses}
                    currentSliceKey={route.sliceKey}
                    onOpen={openSlice}
                  />
                )}
                {!board.data && !board.error && <BoardSkeleton />}
              </>
            )}

            {openTicket && meta.data && (
              <Drawer
                slice={openTicket}
                statuses={meta.data.statuses}
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

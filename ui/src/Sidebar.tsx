// Every repo in the database, and the plans under it.
//
// The database spans every repo on the machine (rule: one database, every worktree),
// and nothing before this showed that. The sidebar is the first place the scope of
// what `aip` already knows becomes visible.

import { useMemo } from "react";

import { ago, statusColour } from "./format";
import { useStored } from "./hooks";
import { Chevron, Logo } from "./icons";
import { navigate, planPath } from "./router";
import type { PlanSummary, RepoSummary } from "./types";

interface Props {
  repos: RepoSummary[];
  plans: PlanSummary[];
  currentSlug: string | undefined;
  search: string;
  onSearch: (value: string) => void;
}

export function Sidebar({ repos, plans, currentSlug, search, onSearch }: Props) {
  const [collapsed, setCollapsed] = useStored<string[]>("ai-planner.collapsed-repos", []);

  const needle = search.trim().toLowerCase();
  const grouped = useMemo(() => {
    const byRepo = new Map<number, PlanSummary[]>();
    for (const plan of plans) {
      if (
        needle &&
        !plan.title.toLowerCase().includes(needle) &&
        !plan.slug.includes(needle) &&
        !plan.repo_name.toLowerCase().includes(needle) &&
        !(plan.ticket_key ?? "").toLowerCase().includes(needle)
      ) {
        continue;
      }
      const list = byRepo.get(plan.repo_id);
      if (list) list.push(plan);
      else byRepo.set(plan.repo_id, [plan]);
    }
    return byRepo;
  }, [plans, needle]);

  const toggle = (key: string) =>
    setCollapsed(
      collapsed.includes(key) ? collapsed.filter((k) => k !== key) : [...collapsed, key],
    );

  // A repo with nothing left after filtering is noise, so it goes - but only while a
  // search is active. With no search, an empty repo is a fact worth seeing.
  const visible = repos.filter((repo) => !needle || (grouped.get(repo.id)?.length ?? 0) > 0);

  return (
    <nav className="sidebar" aria-label="Plans">
      <div className="sidebar-head">
        <div className="wordmark">
          <Logo size={15} />
          ai-planner
        </div>
        <input
          className="search"
          type="search"
          value={search}
          placeholder="Filter plans…"
          aria-label="Filter plans"
          onChange={(event) => onSearch(event.target.value)}
        />
      </div>

      <div className="sidebar-scroll">
        {visible.length === 0 && (
          <p className="column-empty">
            {needle ? "Nothing matches." : "No repos registered yet."}
          </p>
        )}

        {visible.map((repo) => {
          const items = grouped.get(repo.id) ?? [];
          // A search overrides a collapse - hiding the thing you just searched for
          // would be perverse.
          const open = needle !== "" || !collapsed.includes(repo.key);

          return (
            <div className="repo-group" key={repo.id}>
              <button
                className="repo-row"
                onClick={() => toggle(repo.key)}
                aria-expanded={open}
                title={repo.key}
              >
                <Chevron size={11} className={`chevron${open ? " is-open" : ""}`} />
                <span className="repo-name">{repo.name}</span>
                {repo.open_questions > 0 && (
                  <span className="count questions-pill" title={`${repo.open_questions} open questions`}>
                    {repo.open_questions}?
                  </span>
                )}
                <span className="count">{items.length}</span>
              </button>

              {open &&
                items.map((plan) => (
                  <PlanRow key={plan.id} plan={plan} current={plan.slug === currentSlug} />
                ))}

              {open && items.length === 0 && <p className="column-empty">No plans.</p>}
            </div>
          );
        })}
      </div>
    </nav>
  );
}

function PlanRow({ plan, current }: { plan: PlanSummary; current: boolean }) {
  const percent = plan.percent ?? 0;
  return (
    <button
      className={`plan-row${current ? " is-current" : ""}`}
      onClick={() => navigate(planPath(plan.slug))}
      aria-current={current ? "page" : undefined}
    >
      <span className="plan-row-top">
        <span className="dot" style={{ background: statusColour(plan.status) }} />
        <span className="plan-title" title={plan.title}>
          {plan.title}
        </span>
      </span>
      <span className="plan-row-meta">
        {plan.slices > 0 ? (
          <>
            <span className="bar">
              <span style={{ width: `${percent}%` }} />
            </span>
            <span>
              {plan.done}/{plan.slices}
            </span>
          </>
        ) : (
          <span className="bar" />
        )}
        <span>{ago(plan.last_activity ?? plan.updated_at)}</span>
      </span>
    </button>
  );
}

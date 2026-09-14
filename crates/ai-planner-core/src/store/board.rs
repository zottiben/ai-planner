//! The shapes a board reads.
//!
//! A kanban view asks three questions the CLI never had to: what repos exist, how far
//! along is each plan, and what belongs in each column. They live here rather than in
//! the server so that the SQL stays next to the schema it depends on, and so the roll-ups
//! are computed in one pass instead of one query per card.

use std::collections::HashMap;

use rusqlite::Row;
use serde::{Deserialize, Serialize};

use super::{PlanFilter, Store};
use crate::error::Result;
use crate::model::{LogEntry, Plan, Slice, Status};

/// A repo as the sidebar shows it: the row, plus enough counts to decide whether it is
/// worth expanding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoSummary {
    pub id: i64,
    pub key: String,
    pub name: String,
    pub remote_url: Option<String>,
    pub main_path: Option<String>,
    pub plans: i64,
    /// Plans that are neither done nor deliberately dropped.
    pub unfinished_plans: i64,
    pub open_questions: i64,
    pub last_activity: Option<String>,
}

/// A plan with the progress roll-ups `v_plans` computes. Flattened on the wire so the
/// client sees one object, not a plan wrapped in a envelope of counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanSummary {
    #[serde(flatten)]
    pub plan: Plan,
    pub slices: i64,
    pub done: i64,
    /// `None` when the plan has no slices yet - which is different from 0%.
    pub percent: Option<i64>,
    pub open_questions: i64,
    pub last_activity: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardColumn {
    pub status: Status,
    pub slices: Vec<Slice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub plan: Plan,
    /// Every status in `Status::ALL` order, empty columns included. A column that
    /// disappears when it empties is a column you cannot drag a card into.
    pub columns: Vec<BoardColumn>,
}

/// One ticket, opened. The plan and repo names ride along because the drawer is
/// deep-linkable and may be the first thing a session loads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SliceDetail {
    pub slice: Slice,
    pub plan_id: i64,
    pub plan_slug: String,
    pub plan_title: String,
    pub repo: String,
    pub log: Vec<LogEntry>,
}

impl Store {
    pub fn repo_summaries(&self) -> Result<Vec<RepoSummary>> {
        let unfinished = status_list(&Status::UNFINISHED);
        let conn = self.db.conn();
        let mut stmt = conn.prepare(&format!(
            "SELECT r.id, r.key, r.name, r.remote_url, r.main_path,
                    (SELECT COUNT(*) FROM plan p WHERE p.repo_id = r.id),
                    (SELECT COUNT(*) FROM plan p
                      WHERE p.repo_id = r.id AND p.status IN ({unfinished})),
                    (SELECT COUNT(*) FROM question q JOIN plan p ON p.id = q.plan_id
                      WHERE p.repo_id = r.id AND q.status = 'open'),
                    (SELECT MAX(l.at) FROM log l JOIN plan p ON p.id = l.plan_id
                      WHERE p.repo_id = r.id)
             FROM repo r
             ORDER BY r.name"
        ))?;
        let rows = stmt.query_map([], |row| {
            Ok(RepoSummary {
                id: row.get(0)?,
                key: row.get(1)?,
                name: row.get(2)?,
                remote_url: row.get(3)?,
                main_path: row.get(4)?,
                plans: row.get(5)?,
                unfinished_plans: row.get(6)?,
                open_questions: row.get(7)?,
                last_activity: row.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    /// `list_plans` decides which plans and in what order; this only attaches the
    /// counts. Two queries total, however many plans come back - the sidebar renders
    /// every repo on the machine, so one query per plan would be felt.
    pub fn plan_summaries(&self, filter: &PlanFilter) -> Result<Vec<PlanSummary>> {
        let plans = self.list_plans(filter)?;
        if plans.is_empty() {
            return Ok(Vec::new());
        }
        let mut counts = self.plan_counts()?;
        Ok(plans
            .into_iter()
            .map(|plan| {
                let c = counts.remove(&plan.id).unwrap_or_default();
                summarise(plan, c)
            })
            .collect())
    }

    pub fn plan_summary(&self, plan_id: i64) -> Result<PlanSummary> {
        let plan = self.get_plan(plan_id)?;
        let c = self.plan_counts()?.remove(&plan_id).unwrap_or_default();
        Ok(summarise(plan, c))
    }

    /// Slices grouped into every status, in `Status::ALL` order.
    pub fn board(&self, plan_id: i64) -> Result<Board> {
        let plan = self.get_plan(plan_id)?;
        let slices = self.slices(plan_id)?;
        Ok(Board {
            plan,
            columns: columns(slices),
        })
    }

    /// Every slice of every plan in a repo, on one board. Slice keys only mean
    /// something inside their plan, so this is a view mode rather than the default
    /// (D6) - the caller is expected to show which plan each card belongs to.
    pub fn repo_board(&self, repo_id: i64) -> Result<Vec<BoardColumn>> {
        let plans = self.list_plans(&PlanFilter {
            repo_id: Some(repo_id),
            ..Default::default()
        })?;
        let mut all = Vec::new();
        for plan in &plans {
            all.extend(self.slices(plan.id)?);
        }
        Ok(columns(all))
    }

    pub fn slice_detail(&self, slice_id: i64, log_limit: Option<i64>) -> Result<SliceDetail> {
        let slice = self.slice_by_id(slice_id)?;
        let plan = self.get_plan(slice.plan_id)?;
        Ok(SliceDetail {
            plan_id: plan.id,
            plan_slug: plan.slug,
            plan_title: plan.title,
            repo: plan.repo_name,
            log: self.slice_log(slice_id, log_limit)?,
            slice,
        })
    }

    fn plan_counts(&self) -> Result<HashMap<i64, Counts>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT p.id,
                    (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id),
                    (SELECT COUNT(*) FROM slice s WHERE s.plan_id = p.id AND s.status = 'done'),
                    (SELECT COUNT(*) FROM question q
                      WHERE q.plan_id = p.id AND q.status = 'open'),
                    (SELECT MAX(l.at) FROM log l WHERE l.plan_id = p.id)
             FROM plan p",
        )?;
        let rows = stmt.query_map([], row_to_counts)?;
        let mut out = HashMap::new();
        for row in rows {
            let (id, counts) = row?;
            out.insert(id, counts);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Default)]
struct Counts {
    slices: i64,
    done: i64,
    open_questions: i64,
    last_activity: Option<String>,
}

fn row_to_counts(row: &Row<'_>) -> rusqlite::Result<(i64, Counts)> {
    Ok((
        row.get(0)?,
        Counts {
            slices: row.get(1)?,
            done: row.get(2)?,
            open_questions: row.get(3)?,
            last_activity: row.get(4)?,
        },
    ))
}

fn summarise(plan: Plan, c: Counts) -> PlanSummary {
    PlanSummary {
        slices: c.slices,
        done: c.done,
        percent: percent(c.done, c.slices),
        open_questions: c.open_questions,
        last_activity: c.last_activity,
        plan,
    }
}

fn columns(slices: Vec<Slice>) -> Vec<BoardColumn> {
    let mut columns: Vec<BoardColumn> = Status::ALL
        .iter()
        .map(|status| BoardColumn {
            status: *status,
            slices: Vec::new(),
        })
        .collect();
    for slice in slices {
        if let Some(col) = columns.iter_mut().find(|c| c.status == slice.status) {
            col.slices.push(slice);
        }
    }
    columns
}

fn percent(done: i64, total: i64) -> Option<i64> {
    if total == 0 {
        return None;
    }
    Some(((done as f64 / total as f64) * 100.0).round() as i64)
}

fn status_list(statuses: &[Status]) -> String {
    statuses
        .iter()
        .map(|s| format!("'{}'", s.as_str()))
        .collect::<Vec<_>>()
        .join(",")
}

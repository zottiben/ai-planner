use rusqlite::{params, OptionalExtension, Row};

use super::Store;
use crate::error::{Error, Result};
use crate::model::{
    Decision, DecisionStatus, Gotcha, LogEntry, LogKind, Plan, PlanBundle, PlanSource, Question,
    Renders, Section, Slice, Status,
};
use crate::util::{now, slugify, ticket_key};

#[derive(Debug, Clone, Default)]
pub struct NewPlan {
    pub repo_id: i64,
    pub title: String,
    pub slug: Option<String>,
    pub status: Option<Status>,
    pub summary: Option<String>,
    pub ticket_key: Option<String>,
    pub ticket_url: Option<String>,
    pub base_branch: Option<String>,
    pub owner: Option<String>,
    pub raw_md: Option<String>,
    pub source_path: Option<String>,
    /// Skip the starter sections. The importer supplies its own.
    pub bare: bool,
}

#[derive(Debug, Clone, Default)]
pub struct PlanFilter {
    pub repo_id: Option<i64>,
    pub statuses: Vec<Status>,
    pub query: Option<String>,
}

/// A patch over a plan's header. `None` leaves a field alone.
#[derive(Debug, Clone, Default)]
pub struct PlanUpdate {
    pub title: Option<String>,
    pub summary: Option<String>,
    pub ticket_key: Option<String>,
    pub ticket_url: Option<String>,
    pub base_branch: Option<String>,
    pub owner: Option<String>,
}

/// One write to one section. `expect_rev` opts into the conflict check: pass what you
/// read, and a concurrent edit is refused instead of overwritten (D4).
#[derive(Debug, Clone, Default)]
pub struct SectionWrite<'a> {
    pub key: &'a str,
    pub title: Option<&'a str>,
    pub body: &'a str,
    pub renders: Option<Renders>,
    pub ord: Option<i64>,
    pub expect_rev: Option<i64>,
}

/// The skeleton a hand-authored plan starts with. It mirrors the spine every real
/// plan already follows (section 2b of the build plan), so an agent filling one in
/// produces a document that looks like the ones before it.
const STARTER_SECTIONS: &[(&str, &str, Renders)] = &[
    ("outcome", "Outcome", Renders::Body),
    ("grounding", "Grounding", Renders::Body),
    ("sources", "Sources", Renders::Sources),
    ("decisions", "Decisions", Renders::Decisions),
    ("slices", "Delivery slices", Renders::Slices),
    ("questions", "Open questions", Renders::Questions),
    ("gotchas", "Gotchas", Renders::Gotchas),
    ("log", "Progress log", Renders::Log),
];

impl Store {
    pub fn create_plan(&mut self, new: NewPlan) -> Result<Plan> {
        let title = new.title.trim().to_string();
        if title.is_empty() {
            return Err(Error::invalid("a plan needs a title"));
        }
        let ticket = new.ticket_key.or_else(|| ticket_key(&title));
        // A ticketed plan is keyed by its ticket, because that is what branches and
        // filenames carry and therefore what resolution matches on (D7).
        let slug = new
            .slug
            .map(|s| slugify(&s))
            .filter(|s| !s.is_empty())
            .or_else(|| ticket.as_deref().map(slugify))
            .unwrap_or_else(|| slugify(&title));
        if slug.is_empty() {
            return Err(Error::invalid("could not derive a slug - pass --slug"));
        }

        let exists: Option<i64> = self
            .db
            .conn()
            .query_row(
                "SELECT id FROM plan WHERE repo_id = ?1 AND slug = ?2",
                params![new.repo_id, slug],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(Error::DuplicatePlan(slug));
        }

        let status = new.status.unwrap_or(Status::Draft);
        let ts = now();
        let bare = new.bare;
        let id = self.db.write(|tx| {
            tx.execute(
                "INSERT INTO plan
                   (repo_id, slug, title, status, summary, ticket_key, ticket_url,
                    base_branch, owner, raw_md, source_path, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?12)",
                params![
                    new.repo_id,
                    slug,
                    title,
                    status,
                    new.summary,
                    ticket,
                    new.ticket_url,
                    new.base_branch,
                    new.owner,
                    new.raw_md,
                    new.source_path,
                    ts,
                ],
            )?;
            let id = tx.last_insert_rowid();
            if !bare {
                for (ord, (key, title, renders)) in STARTER_SECTIONS.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO plan_section (plan_id, ord, key, title, renders, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
                        params![id, (ord as i64 + 1) * 10, key, title, renders.as_str(), ts],
                    )?;
                }
            }
            Ok(id)
        })?;

        self.get_plan(id)
    }

    pub fn get_plan(&self, id: i64) -> Result<Plan> {
        self.db
            .conn()
            .query_row(&format!("{PLAN_SELECT} WHERE p.id = ?1"), [id], row_to_plan)
            .optional()?
            .ok_or_else(|| Error::NoSuchPlan(id.to_string()))
    }

    /// Resolve a plan the way a person refers to one: by slug, ticket key, numeric id,
    /// or a distinctive part of the title. Ambiguity is an error, never a guess.
    pub fn find_plan(&self, needle: &str, repo_id: Option<i64>) -> Result<Plan> {
        let needle = needle.trim();
        if needle.is_empty() {
            return Err(Error::NoSuchPlan(needle.to_string()));
        }
        let mut candidates = self.match_plans(needle, repo_id)?;
        // A repo-scoped hit beats a global one, so working inside a repo does the
        // right thing without a flag.
        if candidates.len() > 1 && repo_id.is_some() {
            let scoped: Vec<Plan> = candidates
                .iter()
                .filter(|p| Some(p.repo_id) == repo_id)
                .cloned()
                .collect();
            if !scoped.is_empty() {
                candidates = scoped;
            }
        }
        match candidates.len() {
            0 => Err(Error::NoSuchPlan(needle.to_string())),
            1 => Ok(candidates.remove(0)),
            n => {
                let names = candidates
                    .iter()
                    .take(5)
                    .map(|p| format!("{}/{}", p.repo_name, p.slug))
                    .collect::<Vec<_>>()
                    .join(", ");
                Err(Error::AmbiguousPlan(needle.to_string(), n, names))
            }
        }
    }

    fn match_plans(&self, needle: &str, repo_id: Option<i64>) -> Result<Vec<Plan>> {
        let slug = slugify(needle);
        let ticket = ticket_key(needle);
        let numeric: Option<i64> = needle.parse().ok();
        let like = format!("%{}%", needle.to_lowercase());

        let conn = self.db.conn();
        // Ordered by precision: exact slug, then ticket, then id, then title contains.
        let mut stmt = conn.prepare(&format!(
            "{PLAN_SELECT}
             WHERE (?1 IS NOT NULL AND p.slug = ?1)
                OR (?2 IS NOT NULL AND UPPER(p.ticket_key) = ?2)
                OR (?3 IS NOT NULL AND p.id = ?3)
                OR LOWER(p.title) LIKE ?4
                OR p.slug LIKE ?4
             ORDER BY
                CASE WHEN p.slug = ?1 THEN 0
                     WHEN UPPER(p.ticket_key) = ?2 THEN 1
                     WHEN p.id = ?3 THEN 2
                     ELSE 3 END,
                p.updated_at DESC"
        ))?;
        let rows = stmt.query_map(params![slug, ticket, numeric, like], row_to_plan)?;
        let all: Vec<Plan> = rows.collect::<rusqlite::Result<Vec<_>>>()?;

        // An exact hit is never ambiguous, whatever else also matched loosely.
        let exact: Vec<Plan> = all
            .iter()
            .filter(|p| {
                p.slug == slug
                    || p.ticket_key.as_deref().map(str::to_uppercase) == ticket
                    || Some(p.id) == numeric
            })
            .filter(|p| repo_id.is_none() || Some(p.repo_id) == repo_id || exact_is_global(needle))
            .cloned()
            .collect();
        if exact.len() == 1 {
            return Ok(exact);
        }
        if exact.len() > 1 {
            return Ok(exact);
        }
        Ok(all)
    }

    pub fn list_plans(&self, filter: &PlanFilter) -> Result<Vec<Plan>> {
        let conn = self.db.conn();
        let mut sql = format!("{PLAN_SELECT} WHERE 1=1");
        if filter.repo_id.is_some() {
            sql.push_str(" AND p.repo_id = ?1");
        }
        if !filter.statuses.is_empty() {
            let list = filter
                .statuses
                .iter()
                .map(|s| format!("'{}'", s.as_str()))
                .collect::<Vec<_>>()
                .join(",");
            sql.push_str(&format!(" AND p.status IN ({list})"));
        }
        if let Some(q) = &filter.query {
            let esc = q.to_lowercase().replace('\'', "''");
            sql.push_str(&format!(
                " AND (LOWER(p.title) LIKE '%{esc}%' OR p.slug LIKE '%{esc}%' \
                   OR LOWER(COALESCE(p.ticket_key,'')) LIKE '%{esc}%')"
            ));
        }
        sql.push_str(
            " ORDER BY CASE p.status
                         WHEN 'active' THEN 0 WHEN 'in_review' THEN 1 WHEN 'blocked' THEN 2
                         WHEN 'ready' THEN 3 WHEN 'draft' THEN 4 WHEN 'deferred' THEN 5
                         ELSE 6 END,
                       p.updated_at DESC",
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = match filter.repo_id {
            Some(id) => stmt.query_map([id], row_to_plan)?,
            None => stmt.query_map([], row_to_plan)?,
        };
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn set_plan_status(&mut self, plan: &Plan, status: Status) -> Result<Plan> {
        if plan.status == status {
            return Ok(plan.clone());
        }
        let id = plan.id;
        let from = plan.status;
        let me = self.actor.clone();
        self.db.write(|tx| {
            tx.execute(
                "UPDATE plan SET status = ?2, updated_at = ?3, rev = rev + 1 WHERE id = ?1",
                params![id, status, now()],
            )?;
            super::notes::insert_log(
                tx,
                &me,
                id,
                None,
                LogKind::Status,
                &format!("plan {from} -> {status}"),
            )?;
            Ok(())
        })?;
        self.get_plan(id)
    }

    /// Patch the header fields; only what is passed moves.
    pub fn update_plan(&mut self, plan: &Plan, patch: PlanUpdate) -> Result<Plan> {
        let id = plan.id;
        self.db.write(|tx| {
            tx.execute(
                "UPDATE plan SET
                     title       = COALESCE(?2, title),
                     summary     = COALESCE(?3, summary),
                     ticket_url  = COALESCE(?4, ticket_url),
                     ticket_key  = COALESCE(?5, ticket_key),
                     base_branch = COALESCE(?6, base_branch),
                     owner       = COALESCE(?7, owner),
                     updated_at  = ?8,
                     rev         = rev + 1
                 WHERE id = ?1",
                params![
                    id,
                    patch.title,
                    patch.summary,
                    patch.ticket_url,
                    patch.ticket_key,
                    patch.base_branch,
                    patch.owner,
                    now()
                ],
            )?;
            Ok(())
        })?;
        self.get_plan(id)
    }

    pub fn raw_md(&self, plan_id: i64) -> Result<Option<String>> {
        Ok(self
            .db
            .conn()
            .query_row("SELECT raw_md FROM plan WHERE id = ?1", [plan_id], |r| {
                r.get(0)
            })
            .optional()?
            .flatten())
    }

    // -- sections ---------------------------------------------------------------

    pub fn sections(&self, plan_id: i64) -> Result<Vec<Section>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT id, plan_id, ord, key, title, body, renders, rev
             FROM plan_section WHERE plan_id = ?1 ORDER BY ord, id",
        )?;
        let rows = stmt.query_map([plan_id], row_to_section)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn section(&self, plan_id: i64, key: &str) -> Result<Option<Section>> {
        Ok(self
            .db
            .conn()
            .query_row(
                "SELECT id, plan_id, ord, key, title, body, renders, rev
                 FROM plan_section WHERE plan_id = ?1 AND key = ?2",
                params![plan_id, key],
                row_to_section,
            )
            .optional()?)
    }

    /// Create or replace a section's body.
    pub fn set_section(&mut self, plan_id: i64, write: SectionWrite<'_>) -> Result<Section> {
        let existing = self.section(plan_id, write.key)?;
        if let (Some(cur), Some(expected)) = (&existing, write.expect_rev) {
            if cur.rev != expected {
                return Err(Error::Conflict(
                    format!("section {}", write.key),
                    expected,
                    cur.rev,
                ));
            }
        }
        let key = write.key.to_string();
        let title = write
            .title
            .map(str::to_string)
            .or_else(|| existing.as_ref().map(|s| s.title.clone()))
            .unwrap_or_else(|| key.clone());
        let body = write.body.to_string();
        let renders = write
            .renders
            .or_else(|| existing.as_ref().map(|s| s.renders))
            .unwrap_or(Renders::Body);

        self.db.write(|tx| {
            let ord = match write.ord {
                Some(o) => o,
                None => match &existing {
                    Some(s) => s.ord,
                    None => {
                        tx.query_row(
                            "SELECT COALESCE(MAX(ord), 0) + 10 FROM plan_section WHERE plan_id = ?1",
                            [plan_id],
                            |r| r.get::<_, i64>(0),
                        )?
                    }
                },
            };
            tx.execute(
                "INSERT INTO plan_section (plan_id, ord, key, title, body, renders, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                 ON CONFLICT(plan_id, key) DO UPDATE SET
                     ord = excluded.ord, title = excluded.title, body = excluded.body,
                     renders = excluded.renders, updated_at = excluded.updated_at,
                     rev = plan_section.rev + 1",
                params![plan_id, ord, key, title, body, renders.as_str(), now()],
            )?;
            tx.execute(
                "UPDATE plan SET updated_at = ?2 WHERE id = ?1",
                params![plan_id, now()],
            )?;
            Ok(())
        })?;

        self.section(plan_id, &key)?
            .ok_or_else(|| Error::invalid(format!("section {key} vanished")))
    }

    // -- sources ----------------------------------------------------------------

    pub fn add_source(
        &mut self,
        plan_id: i64,
        kind: &str,
        reference: &str,
        note: Option<&str>,
    ) -> Result<()> {
        let (kind, reference, note) = (
            kind.to_string(),
            reference.to_string(),
            note.map(str::to_string),
        );
        self.db.write(|tx| {
            tx.execute(
                "INSERT INTO plan_source (plan_id, kind, ref, note, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![plan_id, kind, reference, note, now()],
            )?;
            Ok(())
        })
    }

    pub fn sources(&self, plan_id: i64) -> Result<Vec<PlanSource>> {
        let conn = self.db.conn();
        let mut stmt = conn.prepare(
            "SELECT id, kind, ref, note FROM plan_source WHERE plan_id = ?1 ORDER BY id",
        )?;
        let rows = stmt.query_map([plan_id], |r| {
            Ok(PlanSource {
                id: r.get(0)?,
                kind: r.get(1)?,
                reference: r.get(2)?,
                note: r.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // -- the whole document -----------------------------------------------------

    pub fn bundle(&self, plan_id: i64) -> Result<PlanBundle> {
        Ok(PlanBundle {
            plan: self.get_plan(plan_id)?,
            sources: self.sources(plan_id)?,
            sections: self.sections(plan_id)?,
            decisions: self.decisions(plan_id)?,
            slices: self.slices(plan_id)?,
            questions: self.questions(plan_id, false)?,
            gotchas: self.gotchas(plan_id)?,
            log: self.log(plan_id, None)?,
        })
    }
}

/// `--plan` with a global-looking reference (`repo/slug`) should be able to reach
/// outside the current repo.
fn exact_is_global(needle: &str) -> bool {
    needle.contains('/')
}

pub(crate) const PLAN_SELECT: &str =
    "SELECT p.id, p.repo_id, r.name, p.slug, p.title, p.status, p.summary,
            p.ticket_key, p.ticket_url, p.base_branch, p.owner, p.source_path,
            p.rev, p.created_at, p.updated_at
     FROM plan p JOIN repo r ON r.id = p.repo_id";

pub(crate) fn row_to_plan(row: &Row<'_>) -> rusqlite::Result<Plan> {
    Ok(Plan {
        id: row.get(0)?,
        repo_id: row.get(1)?,
        repo_name: row.get(2)?,
        slug: row.get(3)?,
        title: row.get(4)?,
        status: row.get(5)?,
        summary: row.get(6)?,
        ticket_key: row.get(7)?,
        ticket_url: row.get(8)?,
        base_branch: row.get(9)?,
        owner: row.get(10)?,
        source_path: row.get(11)?,
        rev: row.get(12)?,
        created_at: row.get(13)?,
        updated_at: row.get(14)?,
    })
}

fn row_to_section(row: &Row<'_>) -> rusqlite::Result<Section> {
    let renders: String = row.get(6)?;
    Ok(Section {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        ord: row.get(2)?,
        key: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        renders: Renders::parse(&renders).unwrap_or(Renders::Body),
        rev: row.get(7)?,
    })
}

// Re-exported row mappers used by the sibling modules.
pub(super) fn row_to_decision(row: &Row<'_>) -> rusqlite::Result<Decision> {
    let status: String = row.get(6)?;
    Ok(Decision {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        ord: row.get(2)?,
        key: row.get(3)?,
        title: row.get(4)?,
        body: row.get(5)?,
        status: DecisionStatus::parse(&status).unwrap_or(DecisionStatus::Agreed),
        superseded_by: row.get(7)?,
        supersede_note: row.get(8)?,
        rev: row.get(9)?,
        decided_at: row.get(10)?,
    })
}

pub(super) fn row_to_question(row: &Row<'_>) -> rusqlite::Result<Question> {
    Ok(Question {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        slice_key: row.get(2)?,
        body: row.get(3)?,
        status: row.get(4)?,
        answer: row.get(5)?,
        asked_at: row.get(6)?,
        answered_at: row.get(7)?,
    })
}

pub(super) fn row_to_gotcha(row: &Row<'_>) -> rusqlite::Result<Gotcha> {
    Ok(Gotcha {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        title: row.get(2)?,
        body: row.get(3)?,
        created_at: row.get(4)?,
    })
}

pub(super) fn row_to_log(row: &Row<'_>) -> rusqlite::Result<LogEntry> {
    let kind: String = row.get(5)?;
    Ok(LogEntry {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        slice_key: row.get(2)?,
        at: row.get(3)?,
        actor: row.get(4)?,
        kind: LogKind::parse(&kind).unwrap_or(LogKind::Progress),
        branch: row.get(6)?,
        worktree_path: row.get(7)?,
        body: row.get(8)?,
    })
}

pub(super) fn row_to_slice(row: &Row<'_>) -> rusqlite::Result<Slice> {
    Ok(Slice {
        id: row.get(0)?,
        plan_id: row.get(1)?,
        ord: row.get(2)?,
        key: row.get(3)?,
        title: row.get(4)?,
        status: row.get(5)?,
        scope_md: row.get(6)?,
        demo_md: row.get(7)?,
        estimate_files: row.get(8)?,
        branch: row.get(9)?,
        base_branch: row.get(10)?,
        pr_url: row.get(11)?,
        worktree_path: row.get(12)?,
        claimed_by: row.get(13)?,
        claimed_at: row.get(14)?,
        blocked_reason: row.get(15)?,
        started_at: row.get(16)?,
        completed_at: row.get(17)?,
        rev: row.get(18)?,
        updated_at: row.get(19)?,
    })
}

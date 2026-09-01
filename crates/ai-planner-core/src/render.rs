//! Rendering a plan back to the markdown it replaced.
//!
//! This is a hard requirement, not a convenience (D3): an agent that wants "the plan"
//! must get the same document it used to `cat`, so nothing about how it *reads* a plan
//! has to change. Only writing moves into the database.

use std::fmt::Write as _;

use crate::model::{DecisionStatus, LogKind, PlanBundle, Renders, Slice, Status};

/// What belongs in the document's progress log. Status flips and decision records are
/// kept out: they are already visible in the slices table and the decisions section,
/// and letting them through buries the session notes that are the point of the log.
/// The full audit trail is still there under `aip logs`.
fn is_narrative(kind: LogKind) -> bool {
    matches!(
        kind,
        LogKind::Progress | LogKind::Verification | LogKind::Blocker | LogKind::Handoff
    )
}

pub fn render_plan(b: &PlanBundle) -> String {
    let mut out = String::with_capacity(4096);

    let _ = writeln!(out, "# {}\n", b.plan.title);

    if let Some(summary) = b
        .plan
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        for line in summary.lines() {
            let _ = writeln!(out, "> {line}");
        }
        out.push('\n');
    }

    let mut meta: Vec<String> = Vec::new();
    if let Some(t) = &b.plan.ticket_key {
        meta.push(match &b.plan.ticket_url {
            Some(url) => format!("**Ticket:** [{t}]({url})"),
            None => format!("**Ticket:** {t}"),
        });
    } else if let Some(url) = &b.plan.ticket_url {
        meta.push(format!("**Ticket:** {url}"));
    }
    meta.push(format!("**Status:** {}", b.plan.status));
    if let Some(base) = &b.plan.base_branch {
        meta.push(format!("**Base branch:** `{base}`"));
    }
    if let Some(owner) = &b.plan.owner {
        meta.push(format!("**Owner:** {owner}"));
    }
    meta.push(format!("**Plan:** `{}`", b.plan.slug));
    let _ = writeln!(out, "{}\n", meta.join(" · "));

    let mut emitted = Vec::new();
    for section in &b.sections {
        let block = render_block(b, section.renders);
        let body = section.body.trim_end();
        // A section with nothing in it yet is a hollow heading, not content.
        if body.is_empty() && block.is_none() {
            continue;
        }
        let _ = writeln!(out, "## {}\n", section.title);
        if !body.is_empty() {
            let _ = writeln!(out, "{body}\n");
        }
        if let Some(block) = block {
            let _ = writeln!(out, "{}\n", block.trim_end());
        }
        emitted.push(section.renders);
    }

    // Anything with content but no section to host it still has to appear, or an
    // imported plan could hide its own slices.
    for renders in [
        Renders::Sources,
        Renders::Decisions,
        Renders::Slices,
        Renders::Questions,
        Renders::Gotchas,
        Renders::Log,
    ] {
        if emitted.contains(&renders) {
            continue;
        }
        if let Some(block) = render_block(b, renders) {
            let _ = writeln!(out, "## {}\n", default_heading(renders));
            let _ = writeln!(out, "{}\n", block.trim_end());
        }
    }

    while out.ends_with("\n\n") {
        out.pop();
    }
    out
}

fn capitalise(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn default_heading(r: Renders) -> &'static str {
    match r {
        Renders::Body => "Notes",
        Renders::Sources => "Sources",
        Renders::Decisions => "Decisions",
        Renders::Slices => "Delivery slices",
        Renders::Questions => "Open questions",
        Renders::Gotchas => "Gotchas",
        Renders::Log => "Progress log",
    }
}

fn render_block(b: &PlanBundle, renders: Renders) -> Option<String> {
    match renders {
        Renders::Body => None,
        Renders::Sources => render_sources(b),
        Renders::Decisions => render_decisions(b),
        Renders::Slices => render_slices(b),
        Renders::Questions => render_questions(b),
        Renders::Gotchas => render_gotchas(b),
        Renders::Log => render_log(b),
    }
}

fn render_sources(b: &PlanBundle) -> Option<String> {
    if b.sources.is_empty() {
        return None;
    }
    let mut out = String::from("| Kind | Reference | Note |\n| --- | --- | --- |\n");
    for s in &b.sources {
        let _ = writeln!(
            out,
            "| {} | {} | {} |",
            s.kind,
            s.reference,
            s.note.as_deref().unwrap_or("")
        );
    }
    Some(out)
}

fn render_decisions(b: &PlanBundle) -> Option<String> {
    if b.decisions.is_empty() {
        return None;
    }
    let mut out = String::new();
    for d in &b.decisions {
        let _ = writeln!(out, "### {} - {}\n", d.key, d.title);
        if d.status != DecisionStatus::Agreed {
            let by = d
                .superseded_by
                .as_deref()
                .map(|k| format!(" by {k}"))
                .unwrap_or_default();
            let head = format!("{}{by}", capitalise(d.status.as_str()));
            match d.supersede_note.as_deref().filter(|n| !n.trim().is_empty()) {
                Some(note) => {
                    let _ = writeln!(out, "> **{head}** - {note}\n");
                }
                None => {
                    let _ = writeln!(out, "> **{head}**\n");
                }
            }
        }
        let body = d.body.trim_end();
        if !body.is_empty() {
            let _ = writeln!(out, "{body}\n");
        }
    }
    Some(out)
}

fn render_slices(b: &PlanBundle) -> Option<String> {
    if b.slices.is_empty() {
        return None;
    }
    let mut out = String::new();

    // The at-a-glance table is the thing the file version never had and the thing
    // you want first when resuming.
    out.push_str("| Slice | Title | Status | Branch | PR |\n| --- | --- | --- | --- | --- |\n");
    for s in &b.slices {
        let _ = writeln!(
            out,
            "| {} | {} | {} {} | {} | {} |",
            s.key,
            s.title,
            s.status.marker(),
            s.status,
            s.branch
                .as_deref()
                .map(|x| format!("`{x}`"))
                .unwrap_or_default(),
            s.pr_url.as_deref().unwrap_or("")
        );
    }
    out.push('\n');

    for s in &b.slices {
        let _ = writeln!(out, "### {} - {}\n", s.key, s.title);
        let _ = writeln!(out, "{}\n", slice_meta_line(s));
        let scope = s.scope_md.trim_end();
        if !scope.is_empty() {
            let _ = writeln!(out, "{scope}\n");
        }
        if let Some(demo) = s
            .demo_md
            .as_deref()
            .map(str::trim)
            .filter(|d| !d.is_empty())
        {
            let _ = writeln!(out, "**Demo:** {demo}\n");
        }
        if let Some(reason) = s
            .blocked_reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
        {
            let _ = writeln!(out, "**Blocked:** {reason}\n");
        }
    }
    Some(out)
}

pub fn slice_meta_line(s: &Slice) -> String {
    let mut bits = vec![format!("**Status:** {} {}", s.status.marker(), s.status)];
    if let Some(n) = s.estimate_files {
        bits.push(format!("**~{n} files**"));
    }
    if let Some(branch) = &s.branch {
        bits.push(format!("**Branch:** `{branch}`"));
    }
    if let Some(base) = &s.base_branch {
        bits.push(format!("**Base:** `{base}`"));
    }
    if let Some(pr) = &s.pr_url {
        bits.push(format!("**PR:** {pr}"));
    }
    if let (Some(by), Some(wt)) = (&s.claimed_by, &s.worktree_path) {
        bits.push(format!("**Claimed:** {by} in `{wt}`"));
    }
    bits.join(" · ")
}

fn render_questions(b: &PlanBundle) -> Option<String> {
    if b.questions.is_empty() {
        return None;
    }
    let mut out = String::new();
    for q in &b.questions {
        let mark = if q.status == "open" { ' ' } else { 'x' };
        let scope = q
            .slice_key
            .as_deref()
            .map(|k| format!("({k}) "))
            .unwrap_or_default();
        let _ = writeln!(out, "- [{mark}] {scope}{}", q.body);
        if let Some(a) = q.answer.as_deref().filter(|a| !a.trim().is_empty()) {
            let _ = writeln!(out, "  - **Answer:** {a}");
        }
    }
    out.push('\n');
    Some(out)
}

fn render_gotchas(b: &PlanBundle) -> Option<String> {
    if b.gotchas.is_empty() {
        return None;
    }
    let mut out = String::new();
    for g in &b.gotchas {
        let _ = writeln!(out, "### {}\n", g.title);
        let body = g.body.trim_end();
        if !body.is_empty() {
            let _ = writeln!(out, "{body}\n");
        }
    }
    Some(out)
}

fn render_log(b: &PlanBundle) -> Option<String> {
    let entries: Vec<_> = b.log.iter().filter(|e| is_narrative(e.kind)).collect();
    if entries.is_empty() {
        return None;
    }
    let mut out = String::new();
    for e in entries {
        let date = e.at.split('T').next().unwrap_or(&e.at);
        let who = e
            .actor
            .as_deref()
            .filter(|a| !a.is_empty())
            .map(|a| format!(" ({a})"))
            .unwrap_or_default();
        let slice = e
            .slice_key
            .as_deref()
            .map(|k| format!("**{k}** "))
            .unwrap_or_default();
        // Continuation lines are indented so a multi-line note stays inside its bullet.
        let body = e.body.trim().replace('\n', "\n  ");
        let _ = writeln!(out, "- {date}{who} - {slice}{body}");
    }
    out.push('\n');
    Some(out)
}

/// A one-line summary for `aip ls` and for the session-start hook.
pub fn plan_headline(b: &PlanBundle) -> String {
    let done = b.slices.iter().filter(|s| s.status == Status::Done).count();
    let total = b.slices.len();
    let progress = if total > 0 {
        format!(" [{done}/{total} slices]")
    } else {
        String::new()
    };
    format!("{} ({}){progress}", b.plan.title, b.plan.status)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    fn plan() -> Plan {
        Plan {
            id: 1,
            repo_id: 1,
            repo_name: "widget".into(),
            slug: "acme-1234".into(),
            title: "ACME-1234 - Reusable Date Range Picker".into(),
            status: Status::Active,
            summary: Some("Living plan.\nOwner: Ben Zotti.".into()),
            ticket_key: Some("ACME-1234".into()),
            ticket_url: Some("https://app.clickup.com/t/1234567/ACME-1234".into()),
            base_branch: Some("master".into()),
            owner: Some("Ben Zotti".into()),
            source_path: None,
            rev: 1,
            created_at: "2026-08-01T00:00:00Z".into(),
            updated_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    fn slice(key: &str, title: &str, status: Status) -> Slice {
        Slice {
            id: 1,
            plan_id: 1,
            ord: 10,
            key: key.into(),
            title: title.into(),
            status,
            scope_md: "Core and the first variant.".into(),
            demo_md: Some("Main Dashboard, pick \"Last quarter\".".into()),
            estimate_files: Some(40),
            branch: Some("feat/date-range-picker".into()),
            base_branch: None,
            pr_url: Some("https://github.com/acme/widget/pull/412".into()),
            worktree_path: None,
            claimed_by: None,
            claimed_at: None,
            blocked_reason: None,
            started_at: None,
            completed_at: None,
            rev: 1,
            updated_at: "2026-08-01T00:00:00Z".into(),
        }
    }

    fn bundle(sections: Vec<Section>, slices: Vec<Slice>) -> PlanBundle {
        PlanBundle {
            plan: plan(),
            sources: vec![],
            sections,
            decisions: vec![],
            slices,
            questions: vec![],
            gotchas: vec![],
            log: vec![],
        }
    }

    fn section(ord: i64, key: &str, title: &str, body: &str, renders: Renders) -> Section {
        Section {
            id: ord,
            plan_id: 1,
            ord,
            key: key.into(),
            title: title.into(),
            body: body.into(),
            renders,
            rev: 1,
        }
    }

    #[test]
    fn a_rendered_plan_reads_like_the_file_it_replaced() {
        let b = bundle(
            vec![
                section(10, "scope", "1. Scope", "A reusable picker.", Renders::Body),
                section(
                    20,
                    "slices",
                    "5. Delivery slices",
                    "Seven PRs.",
                    Renders::Slices,
                ),
            ],
            vec![slice("PR1", "Shared core", Status::InReview)],
        );
        let md = render_plan(&b);

        assert!(md.starts_with("# ACME-1234 - Reusable Date Range Picker\n"));
        assert!(md.contains("> Living plan.\n> Owner: Ben Zotti."));
        assert!(md.contains("[ACME-1234](https://app.clickup.com/t/1234567/ACME-1234)"));
        assert!(md.contains("## 1. Scope\n\nA reusable picker."));
        assert!(md.contains("## 5. Delivery slices\n\nSeven PRs."));
        assert!(md.contains("### PR1 - Shared core"));
        assert!(md.contains("**Demo:** Main Dashboard"));
        assert!(md.contains("`feat/date-range-picker`"));
    }

    #[test]
    fn content_with_no_section_to_host_it_is_still_rendered() {
        // An imported plan whose headings did not include a slices section must not
        // silently hide its slices.
        let b = bundle(
            vec![section(10, "scope", "Scope", "x", Renders::Body)],
            vec![slice("S1", "Blank canvas", Status::Done)],
        );
        let md = render_plan(&b);
        assert!(md.contains("## Delivery slices"));
        assert!(md.contains("### S1 - Blank canvas"));
    }

    #[test]
    fn empty_generated_sections_do_not_leave_hollow_headings() {
        let b = bundle(
            vec![
                section(10, "scope", "Scope", "x", Renders::Body),
                section(20, "log", "Progress log", "", Renders::Log),
            ],
            vec![],
        );
        let md = render_plan(&b);
        assert!(!md.contains("Progress log"));
    }

    #[test]
    fn a_multiline_log_note_stays_inside_its_bullet() {
        let mut b = bundle(vec![], vec![]);
        b.log.push(LogEntry {
            id: 1,
            plan_id: 1,
            slice_key: Some("PR1".into()),
            at: "2026-08-24T03:00:00Z".into(),
            actor: Some("claude".into()),
            kind: LogKind::Progress,
            branch: None,
            worktree_path: None,
            body: "Gates green.\nTypecheck 7/7.".into(),
        });
        let md = render_plan(&b);
        assert!(md.contains("- 2026-08-24 (claude) - **PR1** Gates green.\n  Typecheck 7/7."));
    }
}

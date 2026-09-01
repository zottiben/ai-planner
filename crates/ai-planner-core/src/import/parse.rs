//! Reading the build plans as they are actually written.
//!
//! Every dialect in the sample is supported because it was derived from the sample,
//! not guessed: `## N. Section`, slices keyed `PR1` / `S1` / `M4` / `Phase 0` /
//! `Slice 0` at either heading level, decisions keyed `D1` / `AD-1`, status markers
//! (`✅ DONE`, `⛔ BLOCKED`, `IN REVIEW`, `DELIVERED 2026-07-29`), `**Demo:**` lines,
//! and dated progress-log bullets.
//!
//! Nothing here is destructive: whatever the parser fails to classify stays in a
//! section body, and the raw file is kept verbatim regardless (D5).

use crate::model::{Renders, Status};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSection {
    pub key: String,
    pub title: String,
    pub body: String,
    pub renders: Renders,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedDecision {
    pub key: String,
    pub title: String,
    pub body: String,
    pub section: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedSlice {
    pub key: String,
    pub title: String,
    pub status: Status,
    pub scope: String,
    pub demo: Option<String>,
    pub estimate_files: Option<i64>,
    pub section: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedLog {
    pub at: Option<String>,
    pub body: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParsedPlan {
    pub title: String,
    pub summary: Option<String>,
    pub ticket_key: Option<String>,
    pub ticket_url: Option<String>,
    pub base_branch: Option<String>,
    pub owner: Option<String>,
    pub sections: Vec<ParsedSection>,
    pub decisions: Vec<ParsedDecision>,
    pub slices: Vec<ParsedSlice>,
    pub questions: Vec<String>,
    pub gotchas: Vec<(String, String)>,
    pub log: Vec<ParsedLog>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Heading {
    Slice(String, String),
    Decision(String, String),
    Section(String),
}

struct Block {
    level: usize,
    heading: Heading,
    /// The heading exactly as written. Status markers and file estimates often sit in
    /// the part that splitting the key throws away.
    raw: String,
    body: String,
}

pub fn parse_plan(md: &str) -> ParsedPlan {
    let mut plan = ParsedPlan::default();
    let lines: Vec<&str> = md.lines().collect();

    // Preamble: the title, the blockquote under it, and any `**Field:** value` lines,
    // up to the first section heading.
    let mut i = 0;
    let mut fenced = false;
    let mut preamble: Vec<&str> = Vec::new();
    while i < lines.len() {
        let line = lines[i];
        if is_fence(line) {
            fenced = !fenced;
        }
        if !fenced {
            if let Some((level, text)) = heading_of(line) {
                if level == 1 && plan.title.is_empty() {
                    plan.title = strip_markup(text);
                    i += 1;
                    continue;
                }
                break;
            }
        }
        preamble.push(line);
        i += 1;
    }
    read_preamble(&mut plan, &preamble);

    // Everything from here is a flat list of headed blocks.
    let blocks = split_blocks(&lines[i..]);

    let mut current_section: Option<String> = None;
    for block in blocks {
        match block.heading {
            Heading::Section(title) => {
                let key = section_key(&title, &plan.sections);
                let renders = classify_section(&title);
                current_section = Some(key.clone());
                plan.sections.push(ParsedSection {
                    key,
                    title,
                    body: block.body.trim().to_string(),
                    renders,
                });
            }
            Heading::Decision(key, title) => {
                if plan.decisions.iter().any(|d| d.key == key) {
                    push_back_into_section(
                        &mut plan,
                        &current_section,
                        block.level,
                        &format!("{key} - {title}"),
                        &block.body,
                    );
                    continue;
                }
                plan.decisions.push(ParsedDecision {
                    key,
                    title,
                    body: block.body.trim().to_string(),
                    section: current_section.clone(),
                });
            }
            Heading::Slice(key, title) => {
                if plan.slices.iter().any(|s| s.key == key) {
                    push_back_into_section(
                        &mut plan,
                        &current_section,
                        block.level,
                        &format!("{key} - {title}"),
                        &block.body,
                    );
                    continue;
                }
                let status = slice_status(&block.raw, &key, &block.body);
                let estimate_files =
                    file_estimate(&block.raw).or_else(|| file_estimate(&block.body));
                let (scope, demo) = split_demo(&block.body);
                plan.slices.push(ParsedSlice {
                    key,
                    title: clean_slice_title(&title),
                    status,
                    scope: scope.trim().to_string(),
                    demo,
                    estimate_files,
                    section: current_section.clone(),
                });
            }
        }
    }

    // Sections that host generated content get their bullets lifted into rows, so the
    // questions and the log become addressable instead of prose.
    for section in &mut plan.sections {
        match section.renders {
            Renders::Questions => {
                plan.questions.extend(bullets(&section.body));
                section.body = prose_before_bullets(&section.body);
            }
            Renders::Log => {
                plan.log.extend(log_entries(&section.body));
                section.body = prose_before_bullets(&section.body);
            }
            _ => {}
        }
    }
    mark_container_sections(&mut plan);
    plan
}

/// A heading that turned out to be a duplicate key is not an entity after all; keep it
/// as text so nothing is silently dropped.
fn push_back_into_section(
    plan: &mut ParsedPlan,
    current: &Option<String>,
    level: usize,
    heading: &str,
    body: &str,
) {
    let Some(key) = current else { return };
    let Some(section) = plan.sections.iter_mut().find(|s| &s.key == key) else {
        return;
    };
    let hashes = "#".repeat(level);
    section.body = format!(
        "{}\n\n{hashes} {heading}\n\n{}",
        section.body.trim_end(),
        body.trim()
    )
    .trim()
    .to_string();
}

/// The section that hosts slices (or decisions, or gotchas) renders them in place, so
/// the document keeps its original order (D3).
fn mark_container_sections(plan: &mut ParsedPlan) {
    for (keys, renders) in [
        (
            plan.slices
                .iter()
                .filter_map(|s| s.section.clone())
                .collect::<Vec<_>>(),
            Renders::Slices,
        ),
        (
            plan.decisions
                .iter()
                .filter_map(|d| d.section.clone())
                .collect::<Vec<_>>(),
            Renders::Decisions,
        ),
    ] {
        // The first section that hosts one wins, so a plan whose slices are split
        // across two headings still renders them in one place.
        if let Some(first) = keys.first() {
            if let Some(section) = plan.sections.iter_mut().find(|s| &s.key == first) {
                if section.renders == Renders::Body {
                    section.renders = renders;
                }
            }
        }
    }
}

fn split_blocks(lines: &[&str]) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut fenced = false;
    let mut pending: Option<Block> = None;

    for line in lines {
        if is_fence(line) {
            fenced = !fenced;
        }
        let head = if fenced { None } else { heading_of(line) };

        match head {
            Some((level, text)) if level <= 3 => {
                let classified = classify_heading(text);
                // Level 3 prose headings belong to the section they sit in - splitting
                // on them would shred sections like "2. What already exists".
                if level == 3 && matches!(classified, Heading::Section(_)) {
                    if let Some(b) = pending.as_mut() {
                        b.body.push_str(line);
                        b.body.push('\n');
                        continue;
                    }
                }
                if let Some(b) = pending.take() {
                    blocks.push(b);
                }
                pending = Some(Block {
                    level,
                    heading: classified,
                    raw: text.to_string(),
                    body: String::new(),
                });
            }
            _ => {
                if let Some(b) = pending.as_mut() {
                    b.body.push_str(line);
                    b.body.push('\n');
                }
            }
        }
    }
    if let Some(b) = pending.take() {
        blocks.push(b);
    }
    blocks
}

fn is_fence(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("```") || t.starts_with("~~~")
}

fn heading_of(line: &str) -> Option<(usize, &str)> {
    if !line.starts_with('#') {
        return None;
    }
    let level = line.chars().take_while(|c| *c == '#').count();
    let rest = line[level..].trim();
    if level > 6 || rest.is_empty() {
        return None;
    }
    Some((level, rest))
}

/// `PR1`, `S2`, `M4`, `Phase 0`, `Slice 3`, `D1`, `AD-3` - the keys these plans use.
fn classify_heading(text: &str) -> Heading {
    let stripped = strip_markup(text);
    if let Some((key, rest)) = split_key(&stripped) {
        let upper = key.to_uppercase();
        let letters: String = upper
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        let is_decision = matches!(letters.as_str(), "D" | "AD" | "ADR");
        let is_slice = matches!(
            letters.as_str(),
            "PR" | "S" | "M" | "SLICE" | "MILESTONE" | "PHASE" | "STAGE" | "STEP"
        );
        if is_decision {
            return Heading::Decision(normalise_key(&upper), rest);
        }
        if is_slice {
            return Heading::Slice(normalise_key(&upper), rest);
        }
    }
    Heading::Section(stripped)
}

/// Split `PR1 - Shared core` into (`PR1`, `Shared core`). The separator has to be a
/// spaced dash or a colon, so `AD-1 - Storage` keeps its own dash and a heading like
/// `Storage backends` is not mistaken for a key.
fn split_key(text: &str) -> Option<(String, String)> {
    let (head, rest) = match [" - ", " – ", " — ", ": "]
        .iter()
        .filter_map(|sep| text.find(sep).map(|i| (i, sep.len())))
        .min_by_key(|(i, _)| *i)
    {
        Some((i, len)) => (&text[..i], text[i + len..].trim()),
        None => (text, ""),
    };

    // `PR8 (optional)` is still PR8.
    let head = match head.find('(') {
        Some(i) => head[..i].trim(),
        None => head.trim(),
    };
    if head.is_empty() || head.chars().count() > 12 {
        return None;
    }

    let letters: String = head
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    if letters.is_empty() {
        return None;
    }
    let number = head[letters.len()..].trim().trim_start_matches('-').trim();
    // Only a bare number (optionally with a letter suffix, as in `PR3a`) counts, or
    // `Phase 0 outcome` would masquerade as a second `Phase 0`.
    let mut chars = number.chars();
    let digits: String = chars.by_ref().take_while(|c| c.is_ascii_digit()).collect();
    let suffix: String = number[digits.len()..].to_string();
    if digits.is_empty()
        || suffix.chars().count() > 1
        || suffix.chars().any(|c| !c.is_ascii_alphabetic())
    {
        return None;
    }

    Some((format!("{letters}{digits}{suffix}"), rest.to_string()))
}

/// `PR1` stays `PR1`; `PHASE0` becomes `Phase 0`; `AD3` becomes `AD-3`.
fn normalise_key(upper: &str) -> String {
    let letters: String = upper
        .chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    let digits: String = upper.chars().skip(letters.len()).collect();
    match letters.as_str() {
        "PHASE" => format!("Phase {digits}"),
        "SLICE" => format!("S{digits}"),
        "MILESTONE" => format!("M{digits}"),
        "STAGE" => format!("Stage {digits}"),
        "STEP" => format!("Step {digits}"),
        "AD" | "ADR" => format!("{letters}-{digits}"),
        _ => format!("{letters}{digits}"),
    }
}

fn classify_section(title: &str) -> Renders {
    let t = title.to_lowercase();
    if t.contains("open question") || t.contains("open item") || t.ends_with("questions") {
        Renders::Questions
    } else if t.contains("progress log") || t.contains("change log") || t == "log" {
        Renders::Log
    } else if t.contains("gotcha") {
        Renders::Gotchas
    } else if t.contains("decision log") || t == "decisions" {
        Renders::Decisions
    } else {
        Renders::Body
    }
}

fn section_key(title: &str, existing: &[ParsedSection]) -> String {
    // Drop any leading numbering so a renumbered document keeps its section keys.
    let stripped: String = title
        .trim_start_matches(|c: char| c.is_ascii_digit() || c == '.' || c == ')' || c == ' ')
        .to_string();
    let base = crate::util::slugify(if stripped.is_empty() {
        title
    } else {
        &stripped
    });
    let base = if base.is_empty() {
        "section".to_string()
    } else {
        base
    };
    let mut key = base.clone();
    let mut n = 2;
    while existing.iter().any(|s| s.key == key) {
        key = format!("{base}-{n}");
        n += 1;
    }
    key
}

fn read_preamble(plan: &mut ParsedPlan, lines: &[&str]) {
    let mut quote: Vec<String> = Vec::new();
    for line in lines {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('>') {
            quote.push(rest.trim().to_string());
            continue;
        }
        if let Some((label, value)) = field_line(t) {
            match label.as_str() {
                "ticket" => {
                    plan.ticket_url = url_in(&value);
                    plan.ticket_key = crate::util::ticket_key(&value);
                }
                "owner" => plan.owner = Some(strip_markup(&value)),
                "base branch" | "base" | "branch" => {
                    plan.base_branch = Some(value.trim_matches('`').to_string())
                }
                _ => {}
            }
        }
    }
    if !quote.is_empty() {
        let text = quote.join("\n").trim().to_string();
        // The blockquote often carries the owner and the base branch too.
        if plan.owner.is_none() {
            plan.owner = after_label(&text, "owner:");
        }
        if plan.base_branch.is_none() {
            plan.base_branch = after_label(&text, "base branch:").map(|b| {
                b.trim_matches(|c| c == '`' || c == '.' || c == ',')
                    .to_string()
            });
        }
        plan.summary = Some(text);
    }
    if plan.ticket_key.is_none() {
        plan.ticket_key = crate::util::ticket_key(&plan.title);
    }
}

fn field_line(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("**")?;
    let (label, value) = rest.split_once(":**")?;
    Some((label.trim().to_lowercase(), value.trim().to_string()))
}

fn after_label(text: &str, label: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(label)?;
    let tail = text[at + label.len()..].trim_start();
    let end = tail.find(['\n', '.']).unwrap_or(tail.len());
    let value = tail[..end].trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn url_in(text: &str) -> Option<String> {
    let at = text.find("http")?;
    let tail = &text[at..];
    let end = tail
        .find(|c: char| c.is_whitespace() || c == ')' || c == '>' || c == ']')
        .unwrap_or(tail.len());
    Some(tail[..end].to_string())
}

/// Status markers are matched case-sensitively, because that is how they are written:
/// shouted (`DONE`, `⛔ BLOCKED`, `✅ IN REVIEW`, `- DELIVERED 2026-07-29`) and
/// deliberately distinct from prose. Matching loosely reads a title like "Make the
/// e2e stack complete and honest" as a finished slice.
fn marker_status(text: &str) -> Option<Status> {
    if text.contains('⛔') || text.contains("BLOCKED") {
        return Some(Status::Blocked);
    }
    if text.contains("IN REVIEW") {
        return Some(Status::InReview);
    }
    if text.contains("DONE") || text.contains("DELIVERED") || text.contains('✅') {
        return Some(Status::Done);
    }
    if text.to_lowercase().contains("(optional)") || text.contains("DEFERRED") {
        return Some(Status::Deferred);
    }
    if text.contains("IN PROGRESS") {
        return Some(Status::Active);
    }
    None
}

/// A slice's status comes from its own heading, or from a folded sub-heading that
/// reports on it - "Phase 0 outcome (2026-07-29) - DONE" means Phase 0 is done.
fn slice_status(heading: &str, key: &str, body: &str) -> Status {
    if let Some(status) = marker_status(heading) {
        return status;
    }
    for line in body.lines() {
        if !line.starts_with('#') {
            continue;
        }
        if !line.contains(key) {
            continue;
        }
        if let Some(status) = marker_status(line) {
            return status;
        }
    }
    Status::Ready
}

/// `(~40 files)` -> 40.
fn file_estimate(text: &str) -> Option<i64> {
    let at = text.find('~')?;
    let tail = &text[at + 1..];
    let digits: String = tail.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    let after = tail[digits.len()..].trim_start();
    if !after.to_lowercase().starts_with("file") {
        return None;
    }
    digits.parse().ok()
}

/// Pull the `**Demo:**` line out of a slice body into its own field.
fn split_demo(body: &str) -> (String, Option<String>) {
    let mut demo: Vec<String> = Vec::new();
    let mut scope: Vec<&str> = Vec::new();
    let mut in_demo = false;
    for line in body.lines() {
        let t = line.trim();
        if let Some(rest) = t
            .strip_prefix("**Demo:**")
            .or_else(|| t.strip_prefix("**Demo**:"))
        {
            in_demo = true;
            demo.push(rest.trim().to_string());
            continue;
        }
        if in_demo {
            if t.is_empty() {
                in_demo = false;
                continue;
            }
            demo.push(t.to_string());
            continue;
        }
        scope.push(line);
    }
    let demo = demo.join(" ").trim().to_string();
    (
        scope.join("\n"),
        if demo.is_empty() { None } else { Some(demo) },
    )
}

/// Slice titles carry their status marker and file estimate; both are columns now.
fn clean_slice_title(title: &str) -> String {
    let mut t = title.to_string();
    for marker in ["✅", "⛔", "✔", "❌"] {
        t = t.replace(marker, " ");
    }
    // Drop a shouted status word and whatever date trails it, but only as a suffix.
    for word in ["DELIVERED", "IN REVIEW", "DONE", "BLOCKED", "IN PROGRESS"] {
        if let Some(at) = t.rfind(word) {
            let tail_is_decoration = t[at + word.len()..]
                .chars()
                .all(|c| c.is_ascii_digit() || "-–—/ .:()".contains(c));
            if tail_is_decoration {
                t.truncate(at);
            }
        }
    }
    if let Some(at) = t.find("(~") {
        if let Some(close) = t[at..].find(')') {
            t.replace_range(at..at + close + 1, "");
        }
    }
    // Collapse the run of spaces a stripped marker leaves behind.
    let collapsed = t.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim()
        .trim_end_matches(['-', '·', ','])
        .trim()
        .to_string()
}

fn bullets(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut fenced = false;
    for line in body.lines() {
        if is_fence(line) {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let t = line.trim_start();
        let indented = line.len() - t.len() >= 2;
        if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
            let text = strip_checkbox(rest);
            if indented {
                // A nested bullet continues the one above it.
                if let Some(last) = out.last_mut() {
                    last.push_str("\n  - ");
                    last.push_str(&text);
                    continue;
                }
            }
            out.push(text);
        } else if !t.is_empty() && indented {
            if let Some(last) = out.last_mut() {
                last.push(' ');
                last.push_str(t);
            }
        }
    }
    out
}

fn strip_checkbox(text: &str) -> String {
    let t = text.trim();
    for prefix in ["[ ] ", "[x] ", "[X] "] {
        if let Some(rest) = t.strip_prefix(prefix) {
            return rest.trim().to_string();
        }
    }
    t.to_string()
}

/// Prose that appears before the first bullet stays in the section body.
fn prose_before_bullets(body: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for line in body.lines() {
        let t = line.trim_start();
        if t.starts_with("- ") || t.starts_with("* ") {
            break;
        }
        out.push(line);
    }
    out.join("\n").trim().to_string()
}

/// `- 2026-07-20 - M3 bulk-archive slice completed...` keeps its own date.
fn log_entries(body: &str) -> Vec<ParsedLog> {
    bullets(body)
        .into_iter()
        .map(|text| match leading_date(&text) {
            Some((date, rest)) => ParsedLog {
                at: Some(format!("{date}T00:00:00Z")),
                body: rest,
            },
            None => ParsedLog {
                at: None,
                body: text,
            },
        })
        .collect()
}

fn leading_date(text: &str) -> Option<(String, String)> {
    let t = text.trim();
    if t.len() < 10 {
        return None;
    }
    let head = &t[..10];
    let bytes = head.as_bytes();
    let shaped = bytes.iter().enumerate().all(|(i, b)| match i {
        4 | 7 => *b == b'-',
        _ => b.is_ascii_digit(),
    });
    if !shaped {
        return None;
    }
    let rest = t[10..]
        .trim_start()
        .trim_start_matches(['-', '–', '—'])
        .trim();
    Some((head.to_string(), rest.to_string()))
}

fn strip_markup(text: &str) -> String {
    text.replace("**", "").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_header_yields_the_ticket_owner_and_base_branch() {
        let md = "# ACME-1234 - Reusable Date Range Picker\n\n\
                  **Ticket:** https://app.clickup.com/t/1234567/ACME-1234\n\n\
                  ## 1. Scope\n\nA reusable picker.\n";
        let p = parse_plan(md);
        assert_eq!(p.title, "ACME-1234 - Reusable Date Range Picker");
        assert_eq!(p.ticket_key.as_deref(), Some("ACME-1234"));
        assert_eq!(
            p.ticket_url.as_deref(),
            Some("https://app.clickup.com/t/1234567/ACME-1234")
        );
        assert_eq!(p.sections.len(), 1);
        assert_eq!(p.sections[0].title, "1. Scope");
        assert_eq!(p.sections[0].key, "scope");
    }

    #[test]
    fn a_blockquote_header_yields_the_summary_owner_and_base_branch() {
        let md = "# Accounts V2 - Build Plan\n\n\
                  > Living progress doc. Base branch: `feature/accounts-v2`.\n\
                  > Owner: Ben Zotti. Methodology: dual-track Agile.\n\n\
                  ## Baseline\n\nx\n";
        let p = parse_plan(md);
        assert_eq!(p.owner.as_deref(), Some("Ben Zotti"));
        assert_eq!(p.base_branch.as_deref(), Some("feature/accounts-v2"));
        assert!(p.summary.unwrap().contains("dual-track Agile"));
    }

    #[test]
    fn slices_are_recognised_at_either_heading_level_in_every_dialect() {
        let md = "# P\n\n## 5. Delivery slices\n\nSeven PRs.\n\n\
                  ### PR1 - Shared core (~40 files)\n\nCore work.\n\n\
                  **Demo:** Main Dashboard, pick \"Last quarter\".\n\n\
                  ### S2 - Add entities to the canvas (ACME-1153) - DELIVERED 2026-07-29\n\nDone work.\n\n\
                  ### M2 - Membership foundation  ✅ IN REVIEW\n\nReview work.\n\n\
                  ### Phase 0 - Rebase, re-green, measure\n\nFirst.\n\n\
                  ### PR8 (optional) - Exclude panels\n\nMaybe.\n";
        let p = parse_plan(md);
        let keys: Vec<&str> = p.slices.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["PR1", "S2", "M2", "Phase 0", "PR8"]);

        assert_eq!(p.slices[0].estimate_files, Some(40));
        assert_eq!(
            p.slices[0].demo.as_deref(),
            Some("Main Dashboard, pick \"Last quarter\".")
        );
        assert!(!p.slices[0].scope.contains("Demo"));
        assert_eq!(p.slices[0].title, "Shared core");

        assert_eq!(p.slices[1].status, Status::Done);
        assert_eq!(p.slices[2].status, Status::InReview);
        assert_eq!(p.slices[2].title, "Membership foundation");
        assert_eq!(p.slices[3].status, Status::Ready);
        assert_eq!(p.slices[4].status, Status::Deferred);
        // Lowercase prose must not be read as a marker.
        assert_eq!(
            slice_status("PR9 - Make the stack complete and honest", "PR9", ""),
            Status::Ready
        );
        // A bare tick is a finished slice.
        assert_eq!(
            slice_status("M0 - Grounding & Plan  ✅ (this session)", "M0", ""),
            Status::Done
        );

        // The heading that hosted them renders them in place.
        assert_eq!(p.sections[0].renders, Renders::Slices);
        assert_eq!(p.sections[0].body, "Seven PRs.");
    }

    #[test]
    fn level_two_slices_attach_to_the_section_above_them() {
        let md = "# P\n\n## 4. Delivery approach\n\nStacked.\n\n\
                  ## Slice 0 - Extract a shared AssignToField (enabler)\n\nEnabler.\n\n\
                  ## Slice 1 - Commercial: assign to individuals\n\nWork.\n";
        let p = parse_plan(md);
        let keys: Vec<&str> = p.slices.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, vec!["S0", "S1"]);
        assert_eq!(p.slices[0].section.as_deref(), Some("delivery-approach"));
        assert_eq!(p.sections.len(), 1);
    }

    #[test]
    fn decisions_keep_their_own_numbering_scheme() {
        let md = "# P\n\n## 3. Architecture decisions\n\n\
                  ### AD-1 - Storage: a dedicated table, not a Calculator\n\nBecause.\n\n\
                  ### AD-2 - Content: store the export verbatim\n\nBecause.\n\n\
                  ## 3b. Decisions\n\n### D1 - The value is a specification\n\nBecause.\n";
        let p = parse_plan(md);
        let keys: Vec<&str> = p.decisions.iter().map(|d| d.key.as_str()).collect();
        assert_eq!(keys, vec!["AD-1", "AD-2", "D1"]);
        assert!(p.decisions[0].title.starts_with("Storage"));
    }

    #[test]
    fn prose_subheadings_stay_inside_their_section() {
        let md = "# P\n\n## 2. What already exists\n\nIntro.\n\n\
                  ### Backend bridge\n\nDetail.\n\n\
                  ### Frontend package\n\nMore detail.\n\n\
                  ## 3. Next\n\nx\n";
        let p = parse_plan(md);
        assert_eq!(p.sections.len(), 2);
        assert!(p.sections[0].body.contains("### Backend bridge"));
        assert!(p.sections[0].body.contains("More detail."));
    }

    #[test]
    fn headings_inside_code_fences_are_not_headings() {
        let md = "# P\n\n## 1. Scope\n\n```sh\n# not a heading\n## also not\n```\n\nReal text.\n\n## 2. Next\n\nx\n";
        let p = parse_plan(md);
        assert_eq!(p.sections.len(), 2);
        assert!(p.sections[0].body.contains("# not a heading"));
    }

    #[test]
    fn a_dated_progress_log_keeps_its_own_dates_newest_first() {
        let md = "# P\n\n## Progress log\n\n\
                  - 2026-07-20 - M3 bulk-archive slice completed.\n\
                  - 2026-07-17 - M4 completed. Facility Limits added.\n\
                  - M0 complete: grounded on ClickUp.\n";
        let p = parse_plan(md);
        assert_eq!(p.log.len(), 3);
        assert_eq!(p.log[0].at.as_deref(), Some("2026-07-20T00:00:00Z"));
        assert_eq!(p.log[0].body, "M3 bulk-archive slice completed.");
        assert_eq!(p.log[2].at, None);
        assert_eq!(p.sections[0].renders, Renders::Log);
    }

    #[test]
    fn open_questions_become_rows() {
        let md = "# P\n\n## 6. Open questions\n\nThings needing a call.\n\n\
                  - **Which variant on which surface.** Recommendation: button for page-level filters.\n\
                  - **Exclude...** build it or drop it.\n";
        let p = parse_plan(md);
        assert_eq!(p.questions.len(), 2);
        assert!(p.questions[0].contains("Which variant"));
        assert_eq!(p.sections[0].body, "Things needing a call.");
    }

    #[test]
    fn a_prose_heading_after_a_slice_folds_into_that_slice() {
        // "Phase 0 outcome" is not a second Phase 0 - it is the record of the first
        // one, and it has to end up attached to it rather than becoming a slice or
        // being dropped.
        let md = "# P\n\n## 4. Build plan\n\n\
                  ### Phase 0 - Rebase, re-green, measure\n\nFirst.\n\n\
                  ### Phase 0 outcome (2026-07-29) - DONE\n\nWhat happened.\n";
        let p = parse_plan(md);
        assert_eq!(p.slices.len(), 1);
        assert_eq!(p.slices[0].key, "Phase 0");
        assert!(p.slices[0].scope.contains("Phase 0 outcome"));
        assert!(p.slices[0].scope.contains("What happened."));
        // ...and the outcome's DONE marker is the phase's status.
        assert_eq!(p.slices[0].status, Status::Done);
    }
}

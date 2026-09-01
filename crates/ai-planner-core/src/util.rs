use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

/// ISO-8601 UTC, second precision. Every timestamp in the database uses this so
/// rows sort lexically and read cleanly in TablePlus (D13).
pub fn now() -> String {
    OffsetDateTime::now_utc()
        .replace_nanosecond(0)
        .unwrap_or_else(|_| OffsetDateTime::now_utc())
        .format(&Rfc3339)
        .unwrap_or_default()
}

/// `ACME-1234 - Reusable Date Range Picker` -> `acme-1234-reusable-date-range-picker`
pub fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = true;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Ticket keys as they appear in branches, titles and filenames: `ACME-1234`,
/// `ACME-980`. Returns the first match, upper-cased.
pub fn ticket_key(input: &str) -> Option<String> {
    let bytes: Vec<char> = input.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if !bytes[i].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        let letters = i - start;
        if !(2..=8).contains(&letters) || i >= bytes.len() || bytes[i] != '-' {
            continue;
        }
        let dash = i;
        i += 1;
        let digits_start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i > digits_start {
            let key: String = bytes[start..dash].iter().collect::<String>().to_uppercase();
            let num: String = bytes[digits_start..i].iter().collect();
            return Some(format!("{key}-{num}"));
        }
    }
    None
}

/// Normalise a git remote to a stable, transport-independent key (D2).
/// `git@github.com:org/repo.git` and `https://github.com/org/repo` both become
/// `github.com/org/repo`.
pub fn normalise_remote(url: &str) -> String {
    let mut s = url.trim();
    for prefix in ["ssh://", "git+ssh://", "https://", "http://", "git://"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest;
            break;
        }
    }
    if let Some((userinfo, rest)) = s.split_once('@') {
        if !userinfo.contains('/') {
            s = rest;
        }
    }
    // What is left is `host/path`, `host:path` (scp-style) or `host:port/path`. The
    // colon means different things in the last two, so the split has to look ahead.
    let (host, path) = match s.find([':', '/']) {
        Some(i) if s.as_bytes()[i] == b':' => {
            let after = &s[i + 1..];
            let segment_end = after.find('/').unwrap_or(after.len());
            if !after[..segment_end].is_empty()
                && after[..segment_end].chars().all(|c| c.is_ascii_digit())
            {
                (&s[..i], after[segment_end..].trim_start_matches('/'))
            } else {
                (&s[..i], after)
            }
        }
        Some(i) => (&s[..i], &s[i + 1..]),
        None => (s, ""),
    };

    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let joined = format!("{host}/{path}").to_lowercase();
    joined
        .split('/')
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("/")
}

/// Truncate to `max` display columns, appending an ellipsis when it had to cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(1);
    let mut out: String = s.chars().take(keep).collect();
    out.push('…');
    out
}

/// First non-empty line of a markdown blob, with heading/quote markers stripped.
pub fn first_line(body: &str) -> String {
    body.lines()
        .map(|l| l.trim().trim_start_matches(['#', '>', '-', '*']).trim())
        .find(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_are_kebab_ascii() {
        assert_eq!(
            slugify("ACME-1234 - Reusable Date Range Picker"),
            "acme-1234-reusable-date-range-picker"
        );
        assert_eq!(
            slugify("Canvas Editor - Build Plan"),
            "canvas-editor-build-plan"
        );
        assert_eq!(slugify("  ...  "), "");
    }

    #[test]
    fn ticket_keys_come_out_of_titles_branches_and_filenames() {
        assert_eq!(
            ticket_key("ACME-1234 - Picker").as_deref(),
            Some("ACME-1234")
        );
        assert_eq!(
            ticket_key("feature/acme-1234-csv-export").as_deref(),
            Some("ACME-1234")
        );
        assert_eq!(
            ticket_key("ACME-980-FINDINGS.md").as_deref(),
            Some("ACME-980")
        );
        assert_eq!(ticket_key("feat/date-range-picker"), None);
    }

    #[test]
    fn every_remote_transport_normalises_to_one_key() {
        let expected = "github.com/acme/widget";
        for url in [
            "git@github.com:acme/widget.git",
            "https://github.com/acme/widget.git",
            "https://github.com/acme/widget",
            "ssh://git@github.com/acme/widget.git",
            "ssh://git@github.com:22/acme/widget.git",
            "git://github.com/Acme/widget.git/",
        ] {
            assert_eq!(normalise_remote(url), expected, "for {url}");
        }
    }
}

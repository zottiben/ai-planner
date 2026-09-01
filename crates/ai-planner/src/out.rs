//! Terminal output. Colour only when stdout is a tty, so piped output and agent
//! captures stay clean.

use std::io::IsTerminal;

use ai_planner_core::Status;

pub fn colour() -> bool {
    std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none()
}

pub fn dim(s: &str) -> String {
    if colour() {
        format!("\x1b[2m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn bold(s: &str) -> String {
    if colour() {
        format!("\x1b[1m{s}\x1b[0m")
    } else {
        s.to_string()
    }
}

pub fn status_colour(status: Status, s: &str) -> String {
    if !colour() {
        return s.to_string();
    }
    let code = match status {
        Status::Draft => "2",
        Status::Ready => "0",
        Status::Active => "36",
        Status::InReview => "35",
        Status::Blocked => "31",
        Status::Done => "32",
        Status::Deferred => "2",
    };
    format!("\x1b[{code}m{s}\x1b[0m")
}

pub fn ok(msg: &str) {
    if colour() {
        println!("\x1b[32m✓\x1b[0m {msg}");
    } else {
        println!("ok  {msg}");
    }
}

/// Cut to `max` visible characters, marking that it was cut.
pub fn truncate(s: &str, max: usize) -> String {
    let flat = s.replace('\n', " ");
    if flat.chars().count() <= max {
        return flat;
    }
    let mut out: String = flat.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A left-aligned table that sizes its own columns. Cells may contain colour
/// escapes, so width is measured on the visible text.
pub struct Table {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(headers: &[&str]) -> Table {
        Table {
            headers: headers.iter().map(|h| h.to_string()).collect(),
            rows: Vec::new(),
        }
    }

    pub fn row(&mut self, cells: Vec<String>) {
        self.rows.push(cells);
    }

    pub fn print(&self) {
        let cols = self.headers.len();
        let mut widths: Vec<usize> = self.headers.iter().map(|h| visible_len(h)).collect();
        for row in &self.rows {
            for (i, cell) in row.iter().take(cols).enumerate() {
                widths[i] = widths[i].max(visible_len(cell));
            }
        }

        let header: Vec<String> = self
            .headers
            .iter()
            .enumerate()
            .map(|(i, h)| pad(h, widths[i]))
            .collect();
        println!("{}", dim(header.join("  ").trim_end()));

        for row in &self.rows {
            let line: Vec<String> = row
                .iter()
                .take(cols)
                .enumerate()
                .map(|(i, c)| pad(c, widths[i]))
                .collect();
            println!("{}", line.join("  ").trim_end());
        }
    }
}

fn pad(s: &str, width: usize) -> String {
    let len = visible_len(s);
    if len >= width {
        return s.to_string();
    }
    format!("{s}{}", " ".repeat(width - len))
}

/// Character count with ANSI escapes discounted.
fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for ch in s.chars() {
        if in_escape {
            if ch == 'm' {
                in_escape = false;
            }
            continue;
        }
        if ch == '\x1b' {
            in_escape = true;
            continue;
        }
        len += 1;
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_escapes_do_not_count_towards_column_width() {
        assert_eq!(visible_len("done"), 4);
        assert_eq!(visible_len("\x1b[32mdone\x1b[0m"), 4);
        assert_eq!(visible_len(""), 0);
    }

    #[test]
    fn padding_measures_the_visible_text() {
        assert_eq!(pad("ab", 4), "ab  ");
        assert_eq!(pad("\x1b[32mab\x1b[0m", 4), "\x1b[32mab\x1b[0m  ");
        assert_eq!(pad("abcdef", 4), "abcdef");
    }
}

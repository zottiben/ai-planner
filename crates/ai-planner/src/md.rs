//! Markdown for a human to read.
//!
//! A plan *is* a markdown document, so on a terminal it is worth rendering as one.
//! `gum format` does that (glamour under the hood), and gum is already the styling
//! tool this project reaches for, so the rendering lives outside the binary rather
//! than growing a markdown engine in here.
//!
//! Everything else keeps reading plain markdown. Rendering is a terminal
//! affordance - behind a pipe, under `--json`, or with no gum installed, the bytes
//! are exactly the document they always were.

use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// How markdown leaves this process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// The markdown, unchanged.
    Plain,
    /// Rendered, when `gum` is installed to do it.
    Pretty,
}

impl Mode {
    /// The mode for this process.
    pub fn detect(plain: bool, json: bool) -> Mode {
        decide(
            plain,
            json,
            std::io::stdout().is_terminal(),
            std::env::var_os("NO_COLOR").is_some(),
            std::env::var("AI_PLANNER_MARKDOWN").ok().as_deref(),
        )
    }
}

/// `--json` and `--plain` are the caller being explicit, so they win. `NO_COLOR` and
/// a pipe are the terminal having its say, and `AI_PLANNER_MARKDOWN` overrides both -
/// which is how you render into `less -R` or a screen recording on purpose.
fn decide(plain: bool, json: bool, tty: bool, no_color: bool, setting: Option<&str>) -> Mode {
    if json || plain {
        return Mode::Plain;
    }
    match setting.map(str::trim) {
        Some("plain") | Some("never") | Some("off") | Some("0") => Mode::Plain,
        Some("pretty") | Some("always") | Some("on") | Some("1") => Mode::Pretty,
        _ if tty && !no_color => Mode::Pretty,
        _ => Mode::Plain,
    }
}

/// Print a markdown document.
pub fn print(mode: Mode, md: &str) {
    if mode == Mode::Pretty && format(md) {
        return;
    }
    plain(md);
}

/// Print a markdown document into a scrollable pager. A plan runs to hundreds of
/// lines, and scrolling one is nicer than hunting for it in the scrollback.
pub fn print_paged(mode: Mode, md: &str) {
    if mode == Mode::Pretty && paged(md) {
        return;
    }
    plain(md);
}

fn plain(md: &str) {
    println!("{}", md.trim_end());
}

/// Render straight to the terminal. Returns false when gum could not do it, so the
/// caller can fall back to the markdown itself.
fn format(md: &str) -> bool {
    let mut child = match Command::new("gum")
        .args(["format", "--type", "markdown"])
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .spawn()
    {
        Ok(child) => child,
        // No gum on this machine. The markdown was always readable.
        Err(_) => return false,
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(md.as_bytes());
    }
    // Dropping stdin closed it, which is what tells gum to render and exit. A theme
    // gum does not know is the one real failure here, and it writes nothing before
    // saying so, so falling back cannot double up the output.
    child.wait().map(|s| s.success()).unwrap_or(false)
}

fn paged(md: &str) -> bool {
    let Some(rendered) = render(md) else {
        return false;
    };
    // The content goes on the command line rather than down a pipe: the pager reads
    // its keys from stdin, so stdin has to stay the terminal the reader is typing at.
    // A long plan renders to ~150 KB against an ARG_MAX of 1 MB, and one big enough to
    // break that fails to spawn rather than misbehaving - handled below.
    let child = Command::new("gum")
        .args(["pager", "--soft-wrap", &String::from_utf8_lossy(&rendered)])
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .spawn();
    match child {
        Ok(mut child) => {
            let _ = child.wait();
            // The pager owned the screen; reprinting the plan under it would only be
            // noise, whether the reader quit or the pager fell over.
            true
        }
        // Rendered but unpageable still beats raw markdown.
        Err(_) => {
            let _ = std::io::stdout().write_all(&rendered);
            true
        }
    }
}

/// Render into memory, for handing to the pager. gum sees a pipe here rather than the
/// terminal, which its own themes do not care about.
fn render(md: &str) -> Option<Vec<u8>> {
    let mut child = Command::new("gum")
        .args(["format", "--type", "markdown"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .ok()?;
    // Written from its own thread: a plan renders to more than a pipe buffer holds,
    // so writing and reading have to overlap.
    let mut stdin = child.stdin.take()?;
    let body = md.to_string();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(body.as_bytes());
    });
    let out = child.wait_with_output().ok()?;
    let _ = writer.join();
    out.status.success().then_some(out.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_terminal_gets_rendering_and_a_pipe_does_not() {
        assert_eq!(decide(false, false, true, false, None), Mode::Pretty);
        assert_eq!(decide(false, false, false, false, None), Mode::Plain);
    }

    #[test]
    fn machine_readable_output_is_never_rendered() {
        // An agent parsing this must get the document, whatever the environment says.
        assert_eq!(
            decide(false, true, true, false, Some("always")),
            Mode::Plain
        );
        assert_eq!(
            decide(true, false, true, false, Some("always")),
            Mode::Plain
        );
    }

    #[test]
    fn the_setting_overrides_what_the_terminal_says() {
        assert_eq!(
            decide(false, false, false, false, Some("always")),
            Mode::Pretty
        );
        assert_eq!(
            decide(false, false, true, true, Some("pretty")),
            Mode::Pretty
        );
        assert_eq!(
            decide(false, false, true, false, Some("never")),
            Mode::Plain
        );
    }

    #[test]
    fn no_color_turns_rendering_off() {
        assert_eq!(decide(false, false, true, true, None), Mode::Plain);
    }

    #[test]
    fn an_unknown_setting_falls_back_to_looking_at_the_terminal() {
        assert_eq!(
            decide(false, false, true, false, Some("yes please")),
            Mode::Pretty
        );
        assert_eq!(
            decide(false, false, false, false, Some("yes please")),
            Mode::Plain
        );
    }
}

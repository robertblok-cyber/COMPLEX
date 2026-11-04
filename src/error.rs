// RobertBlok.com — Fullstack development
//
// Compile errors written for humans: where it went wrong, what went wrong,
// and how to fix it.

use std::fmt;
use std::io::IsTerminal;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Clone)]
pub struct CompileError {
    pub file: String,
    pub pos: Pos,
    pub message: String,
    pub hint: Option<String>,
    /// The offending source line, captured when the error is created.
    pub line_text: String,
}

impl CompileError {
    pub fn new(file: &str, source: &str, pos: Pos, message: impl Into<String>) -> Self {
        let line_text = source.lines().nth(pos.line.saturating_sub(1)).unwrap_or("").to_string();
        CompileError { file: file.to_string(), pos, message: message.into(), hint: None, line_text }
    }

    pub fn hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Renders the error, with ANSI colours when `color` is true.
    pub fn render(&self, color: bool) -> String {
        let (red, blue, bold, reset) = if color {
            ("\x1b[31;1m", "\x1b[34;1m", "\x1b[1m", "\x1b[0m")
        } else {
            ("", "", "", "")
        };
        let gutter = self.pos.line.to_string();
        let pad = " ".repeat(gutter.len());
        let line = self.line_text.replace('\t', "    ");
        let caret = " ".repeat(self.pos.col.saturating_sub(1));
        let mut out = format!(
            "{red}error{reset}{bold}: {}{reset}\n{pad}{blue}-->{reset} {}:{}:{}\n{pad} {blue}|{reset}\n{blue}{gutter} |{reset} {line}\n{pad} {blue}|{reset} {caret}{red}^{reset}\n",
            self.message, self.file, self.pos.line, self.pos.col
        );
        if let Some(h) = &self.hint {
            out.push_str(&format!("{pad} {blue}={reset} {bold}hint{reset}: {h}\n"));
        }
        out
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.render(false))
    }
}

/// Prints a list of errors to stderr, coloured when stderr is a terminal.
pub fn report(errors: &[CompileError]) {
    let color = std::io::stderr().is_terminal();
    for e in errors {
        eprintln!("{}", e.render(color));
    }
    let n = errors.len();
    eprintln!("{} error{} found; nothing was run.", n, if n == 1 { "" } else { "s" });
}

/// Returns the candidate closest to `name`, if it is a plausible typo.
pub fn did_you_mean<'a>(name: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(&str, usize)> = None;
    for c in candidates {
        let d = levenshtein(name, c);
        if d <= 2 && d < name.len().max(1) && best.map_or(true, |(_, bd)| d < bd) {
            best = Some((c, d));
        }
    }
    best.map(|(c, _)| c.to_string())
}

fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut cur = vec![i; b.len() + 1];
        for j in 1..=b.len() {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
        }
        prev = cur;
    }
    prev[b.len()]
}

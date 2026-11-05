// RobertBlok.com — Fullstack development
//
// The lexer turns source text into a flat list of tokens.
// It never panics: invalid input becomes a CompileError pointing at the character.

use crate::error::{CompileError, Pos};

#[derive(Debug, Clone, PartialEq)]
pub enum Tok {
    Ident(String),
    Kw(Kw),
    Str(String),
    Int(i64),
    Punct(&'static str),
    Eof,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kw {
    Let,
    Page,
    View,
    Component,
    For,
    In,
    If,
    Else,
    True,
    False,
    Model,
    Seed,
    Log,
}

#[derive(Debug, Clone)]
pub struct Token {
    pub tok: Tok,
    pub pos: Pos,
}

impl Tok {
    /// How the token is shown in error messages.
    pub fn describe(&self) -> String {
        match self {
            Tok::Ident(s) => format!("name '{s}'"),
            Tok::Kw(k) => format!("keyword '{}'", kw_text(*k)),
            Tok::Str(s) => format!("text \"{s}\""),
            Tok::Int(n) => format!("number {n}"),
            Tok::Punct(p) => format!("'{p}'"),
            Tok::Eof => "end of file".into(),
        }
    }
}

pub fn kw_text(k: Kw) -> &'static str {
    match k {
        Kw::Let => "let",
        Kw::Page => "page",
        Kw::View => "view",
        Kw::Component => "component",
        Kw::For => "for",
        Kw::In => "in",
        Kw::If => "if",
        Kw::Else => "else",
        Kw::True => "true",
        Kw::False => "false",
        Kw::Model => "model",
        Kw::Seed => "seed",
        Kw::Log => "log",
    }
}

fn keyword(word: &str) -> Option<Kw> {
    Some(match word {
        "let" => Kw::Let,
        "page" => Kw::Page,
        "view" => Kw::View,
        "component" => Kw::Component,
        "for" => Kw::For,
        "in" => Kw::In,
        "if" => Kw::If,
        "else" => Kw::Else,
        "true" => Kw::True,
        "false" => Kw::False,
        "model" => Kw::Model,
        "seed" => Kw::Seed,
        "log" => Kw::Log,
        _ => return None,
    })
}

const PUNCT2: [&str; 2] = ["==", "!="];
const PUNCT1: [&str; 14] = ["{", "}", "(", ")", "[", "]", ",", ":", ".", "=", "+", "<", ">", "@"];

pub fn lex(file: &str, src: &str) -> Result<Vec<Token>, CompileError> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let (mut i, mut line, mut col) = (0usize, 1usize, 1usize);

    // Moves one character forward while keeping line/column bookkeeping right.
    macro_rules! bump {
        () => {{
            if chars[i] == '\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
            i += 1;
        }};
    }

    while i < chars.len() {
        let c = chars[i];
        let pos = Pos { line, col };

        if c.is_whitespace() {
            bump!();
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                bump!();
            }
            continue;
        }

        if c == '"' {
            bump!();
            let mut s = String::new();
            loop {
                match chars.get(i) {
                    None | Some('\n') => {
                        return Err(CompileError::new(file, src, pos, "text is never closed")
                            .hint("add a closing \" on the same line"));
                    }
                    Some('"') => {
                        bump!();
                        break;
                    }
                    Some('\\') => {
                        bump!();
                        let esc = chars.get(i).copied().unwrap_or(' ');
                        let decoded = match esc {
                            'n' => '\n',
                            't' => '\t',
                            '"' => '"',
                            '\\' => '\\',
                            _ => {
                                return Err(CompileError::new(file, src, Pos { line, col }, format!("unknown escape \\{esc}"))
                                    .hint(r#"supported escapes are \n \t \" \\"#));
                            }
                        };
                        s.push(decoded);
                        bump!();
                    }
                    Some(&ch) => {
                        s.push(ch);
                        bump!();
                    }
                }
            }
            toks.push(Token { tok: Tok::Str(s), pos });
            continue;
        }

        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                bump!();
            }
            let text: String = chars[start..i].iter().collect();
            let n = text.parse::<i64>().map_err(|_| {
                CompileError::new(file, src, pos, "number is too large").hint("numbers must fit in 64 bits")
            })?;
            toks.push(Token { tok: Tok::Int(n), pos });
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                bump!();
            }
            let word: String = chars[start..i].iter().collect();
            let tok = match keyword(&word) {
                Some(k) => Tok::Kw(k),
                None => Tok::Ident(word),
            };
            toks.push(Token { tok, pos });
            continue;
        }

        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        if let Some(p) = PUNCT2.iter().find(|p| **p == two) {
            bump!();
            bump!();
            toks.push(Token { tok: Tok::Punct(p), pos });
            continue;
        }
        if let Some(p) = PUNCT1.iter().find(|p| p.starts_with(c)) {
            bump!();
            toks.push(Token { tok: Tok::Punct(p), pos });
            continue;
        }

        let err = CompileError::new(file, src, pos, format!("unexpected character '{c}'"));
        return Err(if c == '\'' { err.hint("text uses double quotes: \"like this\"") } else { err });
    }

    toks.push(Token { tok: Tok::Eof, pos: Pos { line, col } });
    Ok(toks)
}

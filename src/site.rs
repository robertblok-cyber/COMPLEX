// RobertBlok.com — Fullstack development
//
// Loads a site: every .complex file in a folder becomes one program,
// which is parsed and checked as a whole.

use std::fs;
use std::path::Path;

use crate::ast::Program;
use crate::check::{self, Audit};
use crate::error::CompileError;
use crate::lexer;
use crate::parser;

pub fn load(dir: &Path) -> Result<(Program, Audit), Vec<CompileError>> {
    let mut files: Vec<_> = match fs::read_dir(dir) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "complex"))
            .collect(),
        Err(e) => return Err(vec![io_error(dir, &e.to_string())]),
    };
    files.sort();
    if files.is_empty() {
        return Err(vec![io_error(dir, "no .complex files in this folder")]);
    }

    let mut program = Program::default();
    let mut errors = Vec::new();
    for path in files {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let source = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                errors.push(io_error(&path, &e.to_string()));
                continue;
            }
        };
        let result = lexer::lex(&name, &source).and_then(|toks| parser::parse(&name, &source, toks, &mut program));
        if let Err(e) = result {
            errors.push(e);
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let audit = check::check(&program)?;
    Ok((program, audit))
}

fn io_error(path: &Path, msg: &str) -> CompileError {
    let mut e = CompileError::new(&path.display().to_string(), "", crate::error::Pos { line: 0, col: 0 }, msg);
    e.line_text.clear();
    e
}

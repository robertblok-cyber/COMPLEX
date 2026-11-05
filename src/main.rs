// RobertBlok.com — Fullstack development
//
// complex — one language for the whole web application.
//
//   complex check <dir>            parse and type-check, run nothing
//   complex audit <dir>            list every labelled field and every declassification
//   complex parse <file>           print the syntax tree
//   complex render <dir> <path>    print the HTML for one page
//   complex serve <dir> [addr]     run the development server (default 127.0.0.1:8080)
//   complex cgi <dir>              run once as a CGI program (shared hosting)

mod ast;
mod check;
mod error;
mod http;
mod interp;
mod lexer;
mod parser;
mod site;

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use ast::Program;
use check::Audit;
use http::{Request, Response};
use interp::{RunError, Store};

const VERSION: &str = env!("CARGO_PKG_VERSION");

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("help");
    let result = match cmd {
        "check" => cmd_check(&args),
        "audit" => cmd_audit(&args),
        "parse" => cmd_parse(&args),
        "render" => cmd_render(&args),
        "serve" => cmd_serve(&args),
        "cgi" => cmd_cgi(&args),
        "version" | "--version" | "-V" => {
            println!("complex {VERSION}");
            Ok(())
        }
        _ => {
            print!("{}", USAGE);
            Ok(())
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            if !msg.is_empty() {
                eprintln!("{msg}");
            }
            ExitCode::FAILURE
        }
    }
}

const USAGE: &str = "\
complex — one language for the whole web application

usage:
  complex check  <dir>            parse and type-check, run nothing
  complex audit  <dir>            every labelled field, every declassification, every sink
  complex parse  <file>           print the syntax tree of one file
  complex render <dir> <path>     print the HTML for one page, e.g. /about
  complex serve  <dir> [addr]     development server, default 127.0.0.1:8080
  complex cgi    <dir>            answer one request as a CGI program
  complex version

a site is a folder of .complex files; static files go in <dir>/public
";

fn arg(args: &[String], i: usize, what: &str) -> Result<String, String> {
    args.get(i).cloned().ok_or_else(|| format!("missing argument: {what}\n\n{USAGE}"))
}

/// Loads the site or prints every compile error and fails.
fn load_or_report(dir: &Path) -> Result<(Program, Audit), String> {
    match site::load(dir) {
        Ok(p) => Ok(p),
        Err(errors) => {
            error::report(&errors);
            Err(String::new())
        }
    }
}

fn plural(n: usize) -> &'static str {
    if n == 1 {
        ""
    } else {
        "s"
    }
}

fn cmd_check(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(arg(args, 2, "site folder")?);
    let (program, audit) = load_or_report(&dir)?;
    println!(
        "ok: {} file{}, {} model{}, {} component{}, {} page{}",
        program.files.len(),
        plural(program.files.len()),
        program.models.len(),
        plural(program.models.len()),
        program.components.len(),
        plural(program.components.len()),
        program.pages.len(),
        plural(program.pages.len())
    );
    for p in &program.pages {
        println!("  page {}", p.pattern);
    }
    let n = audit.declassifications.len();
    println!("information flow: {} labelled field{}, {} declassification{} (see `complex audit`)", audit.labeled_fields.len(), plural(audit.labeled_fields.len()), n, plural(n));
    Ok(())
}

/// The audit is the list of facts the checker proved. Everything on it is
/// derived from the source, so it cannot drift from what the code does.
fn cmd_audit(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(arg(args, 2, "site folder")?);
    let (program, audit) = load_or_report(&dir)?;
    println!("complex audit — {}", dir.display());
    println!();
    println!("labelled data");
    if audit.labeled_fields.is_empty() {
        println!("  (none)");
    }
    for f in &audit.labeled_fields {
        println!("  {:<28} {}", format!("{}.{}", f.model, f.field), f.label.name());
    }
    println!();
    println!("declassifications ({})", audit.declassifications.len());
    if audit.declassifications.is_empty() {
        println!("  (none: labelled data never becomes public)");
    }
    for d in &audit.declassifications {
        println!("  {}:{}:{}  {}()  {} -> public", d.file, d.pos.line, d.pos.col, d.how, d.from.name());
        println!("      {}", d.line);
    }
    println!();
    println!(
        "sinks checked: {} view text{}, {} attribute{}, {} branch{}, {} loop{}, {} log line{}",
        audit.sinks_view,
        plural(audit.sinks_view),
        audit.sinks_attr,
        plural(audit.sinks_attr),
        audit.sinks_branch,
        if audit.sinks_branch == 1 { "" } else { "es" },
        audit.sinks_loop,
        plural(audit.sinks_loop),
        audit.sinks_log,
        plural(audit.sinks_log)
    );
    println!();
    println!("result: no labelled value reaches a sink except through the declassifications above");
    println!("        ({} model{}, {} page{}, checked by complex {VERSION})", program.models.len(), plural(program.models.len()), program.pages.len(), plural(program.pages.len()));
    Ok(())
}

fn cmd_parse(args: &[String]) -> Result<(), String> {
    let file = PathBuf::from(arg(args, 2, "file")?);
    let src = fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut program = Program::default();
    let r = lexer::lex(&name, &src).and_then(|t| parser::parse(&name, &src, t, &mut program));
    if let Err(e) = r {
        error::report(&[e]);
        return Err(String::new());
    }
    println!("{program:#?}");
    Ok(())
}

fn cmd_render(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(arg(args, 2, "site folder")?);
    let path = arg(args, 3, "page path, e.g. /")?;
    let (program, _) = load_or_report(&dir)?;
    let store = Store::build(&program).map_err(|e| format!("{e:?}"))?;
    let (res, log) = respond(&program, &store, &dir, "GET", &path);
    for line in log {
        eprintln!("log: {line}");
    }
    if res.status != 200 {
        return Err(format!("{} {}", res.status, String::from_utf8_lossy(&res.body)));
    }
    io::stdout().write_all(&res.body).map_err(|e| e.to_string())
}

fn cmd_serve(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(arg(args, 2, "site folder")?);
    let addr = args.get(3).cloned().unwrap_or_else(|| "127.0.0.1:8080".into());
    let (program, _) = load_or_report(&dir)?;
    let store = Store::build(&program).map_err(|e| format!("{e:?}"))?;
    eprintln!("complex {VERSION} serving {} on http://{addr}/", dir.display());
    for p in &program.pages {
        eprintln!("  http://{addr}{}", p.pattern);
    }
    let dir = Arc::new(dir);
    let program = Arc::new(program);
    let store = Arc::new(store);
    let handler: Arc<http::Handler> = Arc::new(move |req: &Request| {
        let (res, log) = respond(&program, &store, &dir, &req.method, &req.path);
        for line in log {
            eprintln!("{} log {line}", interp::format_utc(interp::unix_now()));
        }
        res
    });
    http::serve(&addr, handler, true).map_err(|e| format!("cannot listen on {addr}: {e}"))
}

/// CGI: the web server passes the request in environment variables and
/// expects headers plus body on stdout. Works on any shared host.
fn cmd_cgi(args: &[String]) -> Result<(), String> {
    let dir = PathBuf::from(arg(args, 2, "site folder")?);
    let method = env::var("REQUEST_METHOD").unwrap_or_else(|_| "GET".into());
    let path = env::var("PATH_INFO").ok().filter(|p| !p.is_empty()).unwrap_or_else(|| "/".into());
    let mut body = Vec::new();
    let _ = io::stdin().read_to_end(&mut body);

    let res = match site::load(&dir) {
        Ok((program, _)) => match Store::build(&program) {
            Ok(store) => respond(&program, &store, &dir, &method, &path).0,
            Err(e) => Response::html(500, error_page("Runtime error", &interp::escape_text(&format!("{e:?}")))),
        },
        Err(errors) => {
            let text: String = errors.iter().map(|e| e.render(false)).collect::<Vec<_>>().join("\n");
            Response::html(500, error_page("Compile error", &format!("<pre>{}</pre>", interp::escape_text(&text))))
        }
    };
    let mut out = io::stdout().lock();
    let _ = write!(out, "Status: {}\r\nContent-Type: {}\r\nX-Content-Type-Options: nosniff\r\n", res.status, res.content_type);
    for (k, v) in &res.extra_headers {
        let _ = write!(out, "{k}: {v}\r\n");
    }
    let _ = out.write_all(b"\r\n");
    let _ = out.write_all(&res.body);
    Ok(())
}

/// Turns a request into a response plus the log lines the page wrote.
fn respond(program: &Program, store: &Store, dir: &Path, method: &str, path: &str) -> (Response, Vec<String>) {
    let mut log = Vec::new();
    if method != "GET" && method != "HEAD" {
        return (Response::text(405, "method not allowed"), log);
    }
    if let Some(res) = static_file(dir, path) {
        return (res, log);
    }
    let Some((page, params)) = interp::route(program, path) else {
        return (Response::html(404, error_page("404", &format!("No page at <code>{}</code>.", interp::escape_text(path)))), log);
    };
    let mut it = interp::Interpreter::new(program, store);
    let res = match it.render_page(page, &params, &mut log) {
        Ok(html) => Response::html(200, html),
        Err(RunError::NotFound(msg)) => Response::html(404, error_page("404", &interp::escape_text(&msg))),
        Err(RunError::Internal(msg)) => Response::html(500, error_page("Runtime error", &interp::escape_text(&msg))),
    };
    (res, log)
}

/// Serves <dir>/public/<path> when it exists and the path stays inside it.
fn static_file(dir: &Path, path: &str) -> Option<Response> {
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.split('/').any(|seg| seg == ".." || seg.is_empty() || seg.starts_with('.')) {
        return None;
    }
    let full = dir.join("public").join(rel);
    let data = fs::read(&full).ok()?;
    let ct = match full.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "css" => "text/css; charset=utf-8",
        "js" => "text/javascript; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        "pdf" => "application/pdf",
        _ => "application/octet-stream",
    };
    Some(Response::file(ct, data))
}

fn error_page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title><style>body{{font:16px/1.5 ui-monospace,monospace;background:#0b0e14;color:#c9d1d9;padding:3rem}}h1{{color:#58a6ff}}code,pre{{color:#ffa657}}</style><h1>{title}</h1><p>{body}</p><p><small>complex {VERSION}</small></p>"
    )
}

// RobertBlok.com — Fullstack development
//
// A tree-walking interpreter: it walks the checked AST and produces HTML.
// Because the checker already proved names and types correct, runtime errors
// here mean a bug in Complex itself, not in the program.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::*;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    Record(Vec<(String, Value)>),
    Secret(Box<Value>),
}

pub struct Interpreter<'a> {
    program: &'a Program,
    scopes: Vec<HashMap<String, Value>>,
    /// Rendered children of the component calls currently being rendered.
    children: Vec<String>,
}

type IResult<T> = Result<T, String>;

/// Finds the page for a path and returns it with the URL parameters it captured.
pub fn route<'a>(program: &'a Program, path: &str) -> Option<(&'a Page, Vec<(String, String)>)> {
    let path = if path.len() > 1 { path.trim_end_matches('/') } else { path };
    let want: Vec<&str> = path.split('/').collect();
    'pages: for page in &program.pages {
        let have: Vec<&str> = page.pattern.split('/').collect();
        if have.len() != want.len() {
            continue;
        }
        let mut params = Vec::new();
        for (h, w) in have.iter().zip(&want) {
            if let Some(name) = h.strip_prefix(':') {
                if w.is_empty() {
                    continue 'pages;
                }
                params.push((name.to_string(), percent_decode(w)));
            } else if h != w {
                continue 'pages;
            }
        }
        return Some((page, params));
    }
    None
}

impl<'a> Interpreter<'a> {
    pub fn new(program: &'a Program) -> Self {
        Interpreter { program, scopes: vec![HashMap::new()], children: Vec::new() }
    }

    /// Renders a full HTML document for a page.
    pub fn render_page(&mut self, page: &Page, params: &[(String, String)]) -> IResult<String> {
        for l in &self.program.lets {
            let v = self.eval(&l.value)?;
            self.bind(&l.name, v);
        }
        self.scopes.push(HashMap::new());
        for (k, v) in params {
            self.bind(k, Value::Text(v.clone()));
        }
        for l in &page.lets {
            let v = self.eval(&l.value)?;
            self.bind(&l.name, v);
        }
        let mut out = String::from("<!doctype html>\n");
        self.nodes(&page.view, &mut out)?;
        self.scopes.pop();
        out.push('\n');
        Ok(out)
    }

    fn bind(&mut self, name: &str, v: Value) {
        self.scopes.last_mut().expect("scope").insert(name.to_string(), v);
    }

    fn lookup(&self, name: &str) -> IResult<Value> {
        self.scopes
            .iter()
            .rev()
            .find_map(|s| s.get(name).cloned())
            .ok_or_else(|| format!("internal: '{name}' is not bound"))
    }

    // ---- rendering ----------------------------------------------------------

    fn nodes(&mut self, nodes: &[Node], out: &mut String) -> IResult<()> {
        for n in nodes {
            self.node(n, out)?;
        }
        Ok(())
    }

    fn node(&mut self, node: &Node, out: &mut String) -> IResult<()> {
        match node {
            Node::Text(e) => {
                let v = self.eval(e)?;
                out.push_str(&escape_text(&to_text(&v)?));
            }
            Node::Children { .. } => {
                let html = self.children.last().cloned().unwrap_or_default();
                out.push_str(&html);
            }
            Node::For { var, list, body, .. } => {
                let items = match self.eval(list)? {
                    Value::List(items) => items,
                    other => return Err(format!("internal: for over {other:?}")),
                };
                for item in items {
                    self.scopes.push(HashMap::new());
                    self.bind(var, item);
                    let r = self.nodes(body, out);
                    self.scopes.pop();
                    r?;
                }
            }
            Node::If { cond, then, otherwise, .. } => match self.eval(cond)? {
                Value::Bool(true) => self.nodes(then, out)?,
                Value::Bool(false) => self.nodes(otherwise, out)?,
                other => return Err(format!("internal: if on {other:?}")),
            },
            Node::Element { name, args, children, .. } => {
                if let Some(comp) = self.program.components.iter().find(|c| &c.name == name) {
                    self.call_component(comp, args, children, out)?;
                } else {
                    self.html_element(name, args, children, out)?;
                }
            }
        }
        Ok(())
    }

    fn html_element(&mut self, name: &str, args: &[Arg], children: &[Node], out: &mut String) -> IResult<()> {
        out.push('<');
        out.push_str(name);
        for arg in args {
            let attr = arg.name.replace('_', "-");
            match self.eval(&arg.value)? {
                Value::Bool(true) => {
                    out.push(' ');
                    out.push_str(&attr);
                }
                Value::Bool(false) => {}
                v => {
                    let mut text = to_text(&v)?;
                    // Defence in depth: the checker rejects literal script URLs,
                    // this catches dynamic ones.
                    if matches!(attr.as_str(), "href" | "src" | "action")
                        && text.trim().to_ascii_lowercase().starts_with("javascript:")
                    {
                        text = "#".into();
                    }
                    out.push_str(&format!(" {attr}=\"{}\"", escape_attr(&text)));
                }
            }
        }
        out.push('>');
        if VOID_TAGS.contains(&name) {
            return Ok(());
        }
        self.nodes(children, out)?;
        out.push_str(&format!("</{name}>"));
        Ok(())
    }

    fn call_component(&mut self, comp: &Component, args: &[Arg], children: &[Node], out: &mut String) -> IResult<()> {
        // Children are rendered in the caller's scope, before entering the component.
        let mut child_html = String::new();
        self.nodes(children, &mut child_html)?;

        let mut frame = HashMap::new();
        for arg in args {
            frame.insert(arg.name.clone(), self.eval(&arg.value)?);
        }
        // A component sees the globals and its own parameters, never the caller's locals.
        let globals = self.scopes[0].clone();
        let saved = std::mem::replace(&mut self.scopes, vec![globals, frame]);
        self.children.push(child_html);
        let r = self.nodes(&comp.body, out);
        self.children.pop();
        self.scopes = saved;
        r
    }

    // ---- expressions --------------------------------------------------------

    fn eval(&mut self, e: &Expr) -> IResult<Value> {
        Ok(match e {
            Expr::Str { value, .. } => Value::Text(value.clone()),
            Expr::Int { value, .. } => Value::Int(*value),
            Expr::Bool { value, .. } => Value::Bool(*value),
            Expr::List { items, .. } => Value::List(items.iter().map(|i| self.eval(i)).collect::<IResult<_>>()?),
            Expr::Record { fields, .. } => Value::Record(
                fields.iter().map(|f| Ok((f.name.clone(), self.eval(&f.value)?))).collect::<IResult<_>>()?,
            ),
            Expr::Name { name, .. } => self.lookup(name)?,
            Expr::Field { target, name, .. } => {
                let (v, secret) = unwrap_secret(self.eval(target)?);
                let field = match v {
                    Value::Record(fields) => fields.into_iter().find(|(n, _)| n == name).map(|(_, v)| v),
                    _ => None,
                };
                rewrap(field.ok_or_else(|| format!("internal: no field '{name}'"))?, secret)
            }
            Expr::Binary { op, left, right, .. } => {
                let (l, ls) = unwrap_secret(self.eval(left)?);
                let (r, rs) = unwrap_secret(self.eval(right)?);
                let v = match (op, &l, &r) {
                    (BinOp::Add, Value::Int(a), Value::Int(b)) => Value::Int(a.saturating_add(*b)),
                    (BinOp::Add, _, _) => Value::Text(to_text(&l)? + &to_text(&r)?),
                    (BinOp::Eq, _, _) => Value::Bool(l == r),
                    (BinOp::NotEq, _, _) => Value::Bool(l != r),
                };
                rewrap(v, ls || rs)
            }
            Expr::Call { name, args, .. } => {
                let vals: Vec<Value> = args.iter().map(|a| self.eval(a)).collect::<IResult<_>>()?;
                builtin(name, vals)?
            }
        })
    }
}

fn builtin(name: &str, mut args: Vec<Value>) -> IResult<Value> {
    Ok(match name {
        "now" => Value::Text(format_utc(unix_now())),
        "year" => Value::Int(civil_from_days(unix_now() / 86_400).0),
        "len" => {
            let (v, secret) = unwrap_secret(args.remove(0));
            let n = match v {
                Value::List(items) => items.len() as i64,
                Value::Text(t) => t.chars().count() as i64,
                other => return Err(format!("internal: len of {other:?}")),
            };
            rewrap(Value::Int(n), secret)
        }
        "upper" => {
            let (v, secret) = unwrap_secret(args.remove(0));
            rewrap(Value::Text(to_text(&v)?.to_uppercase()), secret)
        }
        "secret" => rewrap(args.remove(0), true),
        "masked" => {
            let (v, _) = unwrap_secret(args.remove(0));
            let keep = match args.remove(0) {
                Value::Int(n) => n.max(0) as usize,
                _ => 0,
            };
            let chars: Vec<char> = to_text(&v)?.chars().collect();
            let hidden = chars.len().saturating_sub(keep);
            let shown: String = chars[hidden..].iter().collect();
            Value::Text("*".repeat(hidden) + &shown)
        }
        other => return Err(format!("internal: unknown builtin {other}")),
    })
}

fn unwrap_secret(v: Value) -> (Value, bool) {
    match v {
        Value::Secret(inner) => (*inner, true),
        v => (v, false),
    }
}

fn rewrap(v: Value, secret: bool) -> Value {
    if secret && !matches!(v, Value::Secret(_)) {
        Value::Secret(Box::new(v))
    } else {
        v
    }
}

fn to_text(v: &Value) -> IResult<String> {
    Ok(match v {
        Value::Text(t) => t.clone(),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Secret(_) => return Err("internal: refused to render a secret".into()),
        other => return Err(format!("internal: cannot render {other:?}")),
    })
}

pub fn escape_text(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn escape_attr(s: &str) -> String {
    escape_text(s).replace('"', "&quot;").replace('\'', "&#39;")
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(b) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ---- time without external crates ------------------------------------------

pub fn unix_now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Days since 1970-01-01 to (year, month, day); Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    (y, m, d)
}

pub fn format_utc(secs: i64) -> String {
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    let s = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", s / 3600, (s % 3600) / 60, s % 60)
}

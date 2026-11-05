// RobertBlok.com — Fullstack development
//
// A tree-walking interpreter: it walks the checked AST and produces HTML.
// Because the checker already proved names, types and labels correct,
// an internal error here means a bug in Complex itself, not in the program.
// Labels still travel with values at runtime as defence in depth: a labelled
// value that somehow reaches a sink is refused, never rendered.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::ast::*;
use crate::check::Label;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Text(String),
    Int(i64),
    Bool(bool),
    List(Vec<Value>),
    Record(Vec<(String, Value)>),
    ModelRef(String),
    Labeled(Box<Value>, Label),
}

#[derive(Debug)]
pub enum RunError {
    /// No record for this id: becomes a 404.
    NotFound(String),
    /// A bug in the runtime: becomes a 500.
    Internal(String),
}

type IResult<T> = Result<T, RunError>;

fn internal<T>(msg: impl Into<String>) -> IResult<T> {
    Err(RunError::Internal(msg.into()))
}

/// The in-memory store, filled from `seed` declarations at startup.
pub struct Store {
    records: HashMap<String, Vec<Value>>,
}

impl Store {
    pub fn build(program: &Program) -> IResult<Store> {
        let mut records: HashMap<String, Vec<Value>> = HashMap::new();
        let mut it = Interpreter { program, store: None, scopes: vec![HashMap::new()], children: Vec::new() };
        for seed in &program.seeds {
            let model = program.models.iter().find(|m| m.name == seed.model).ok_or_else(|| RunError::Internal("seed without model".into()))?;
            let Value::List(items) = it.eval(&seed.value)? else { return internal("seed is not a list") };
            let entry = records.entry(seed.model.clone()).or_default();
            for item in items {
                let Value::Record(fields) = item else { return internal("seed item is not a record") };
                // Attach each field's label at the source, as the model declares it.
                let labelled = fields
                    .into_iter()
                    .map(|(name, v)| {
                        let label = model
                            .fields
                            .iter()
                            .find(|f| f.name == name)
                            .and_then(|f| f.label.as_ref())
                            .and_then(|(l, _)| match l.as_str() {
                                "secret" => Some(Label::Secret),
                                "personal" => Some(Label::Personal),
                                _ => None,
                            })
                            .unwrap_or(Label::Public);
                        (name, rewrap(v, label))
                    })
                    .collect();
                entry.push(Value::Record(labelled));
            }
        }
        Ok(Store { records })
    }
}

pub struct Interpreter<'a> {
    program: &'a Program,
    store: Option<&'a Store>,
    scopes: Vec<HashMap<String, Value>>,
    /// Rendered children of the component calls currently being rendered.
    children: Vec<String>,
}

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
    pub fn new(program: &'a Program, store: &'a Store) -> Self {
        Interpreter { program, store: Some(store), scopes: vec![HashMap::new()], children: Vec::new() }
    }

    /// Renders a full HTML document for a page.
    pub fn render_page(&mut self, page: &Page, params: &[(String, String)], log: &mut Vec<String>) -> IResult<String> {
        for l in &self.program.lets {
            let v = self.eval(&l.value)?;
            self.bind(&l.name, v);
        }
        self.scopes.push(HashMap::new());
        for (k, raw) in params {
            let declared = page.params.iter().find(|p| &p.name == k).map(|p| &p.ty);
            let v = match declared {
                Some(TypeExpr::Named(t)) if t == "Int" => match raw.parse::<i64>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => return Err(RunError::NotFound(format!("'{raw}' is not a number"))),
                },
                _ => Value::Text(raw.clone()),
            };
            self.bind(k, v);
        }
        for stmt in &page.stmts {
            match stmt {
                Stmt::Let(l) => {
                    let v = self.eval(&l.value)?;
                    self.bind(&l.name, v);
                }
                Stmt::Log { value, .. } => {
                    let v = self.eval(value)?;
                    log.push(to_text(&v)?);
                }
            }
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
        if let Some(v) = self.scopes.iter().rev().find_map(|s| s.get(name).cloned()) {
            return Ok(v);
        }
        if self.program.models.iter().any(|m| m.name == name) {
            return Ok(Value::ModelRef(name.to_string()));
        }
        internal(format!("'{name}' is not bound"))
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
                    other => return internal(format!("for over {other:?}")),
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
                other => return internal(format!("if on {other:?}")),
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
                    if matches!(attr.as_str(), "href" | "src" | "action") && text.trim().to_ascii_lowercase().starts_with("javascript:") {
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
        let mut child_html = String::new();
        self.nodes(children, &mut child_html)?;
        let mut frame = HashMap::new();
        for arg in args {
            frame.insert(arg.name.clone(), self.eval(&arg.value)?);
        }
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
            Expr::Record { fields, .. } => Value::Record(fields.iter().map(|f| Ok((f.name.clone(), self.eval(&f.value)?))).collect::<IResult<_>>()?),
            Expr::Name { name, .. } => self.lookup(name)?,
            Expr::Field { target, name, .. } => {
                let (v, label) = unwrap(self.eval(target)?);
                match v {
                    Value::Record(fields) => {
                        let f = fields.into_iter().find(|(n, _)| n == name).map(|(_, v)| v);
                        rewrap(f.ok_or_else(|| RunError::Internal(format!("no field '{name}'")))?, label)
                    }
                    Value::ModelRef(m) if name == "all" => {
                        let records = self.store.and_then(|s| s.records.get(&m)).cloned().unwrap_or_default();
                        Value::List(records)
                    }
                    other => return internal(format!("field '{name}' on {other:?}")),
                }
            }
            Expr::Index { target, index, .. } => {
                let Value::ModelRef(m) = self.eval(target)? else { return internal("index on non-model") };
                let Value::Int(id) = self.eval(index)? else { return internal("index is not Int") };
                let records = self.store.and_then(|s| s.records.get(&m));
                // Ids are 1-based positions in the seed list.
                match records.and_then(|r| usize::try_from(id - 1).ok().and_then(|i| r.get(i))) {
                    Some(rec) => rec.clone(),
                    None => return Err(RunError::NotFound(format!("no {m} with id {id}"))),
                }
            }
            Expr::Binary { op, left, right, .. } => {
                let (l, ll) = unwrap(self.eval(left)?);
                let (r, rl) = unwrap(self.eval(right)?);
                let v = match (op, &l, &r) {
                    (BinOp::Add, Value::Int(a), Value::Int(b)) => Value::Int(a.saturating_add(*b)),
                    (BinOp::Add, _, _) => Value::Text(to_text(&l)? + &to_text(&r)?),
                    (BinOp::Eq, _, _) => Value::Bool(l == r),
                    (BinOp::NotEq, _, _) => Value::Bool(l != r),
                };
                rewrap(v, ll.max(rl))
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
            let (v, label) = unwrap(args.remove(0));
            let n = match v {
                Value::List(items) => items.len() as i64,
                Value::Text(t) => t.chars().count() as i64,
                other => return internal(format!("len of {other:?}")),
            };
            rewrap(Value::Int(n), label)
        }
        "upper" => {
            let (v, label) = unwrap(args.remove(0));
            rewrap(Value::Text(to_text(&v)?.to_uppercase()), label)
        }
        "secret" => rewrap(args.remove(0), Label::Secret),
        "reveal" => unwrap(args.remove(0)).0,
        "masked" => {
            let (v, _) = unwrap(args.remove(0));
            let keep = match args.remove(0) {
                Value::Int(n) => n.max(0) as usize,
                _ => 0,
            };
            let chars: Vec<char> = to_text(&v)?.chars().collect();
            let hidden = chars.len().saturating_sub(keep);
            let shown: String = chars[hidden..].iter().collect();
            Value::Text("*".repeat(hidden) + &shown)
        }
        other => return internal(format!("unknown builtin {other}")),
    })
}

fn unwrap(v: Value) -> (Value, Label) {
    match v {
        Value::Labeled(inner, l) => (*inner, l),
        v => (v, Label::Public),
    }
}

fn rewrap(v: Value, label: Label) -> Value {
    let (inner, own) = unwrap(v);
    let l = own.max(label);
    if l == Label::Public {
        inner
    } else {
        Value::Labeled(Box::new(inner), l)
    }
}

fn to_text(v: &Value) -> IResult<String> {
    Ok(match v {
        Value::Text(t) => t.clone(),
        Value::Int(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Labeled(_, l) => return internal(format!("refused to render {} data", l.name())),
        other => return internal(format!("cannot render {other:?}")),
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

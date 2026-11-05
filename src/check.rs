// RobertBlok.com — Fullstack development
//
// The checker runs before anything is served. It resolves every name, infers
// every type, and enforces information flow control.
//
// Every value carries a security label from a three-point lattice:
//
//     Public  <  Personal  <  Secret
//
// A label is attached at the source (a `@personal` or `@secret` field on a
// model, or secret(...)) and flows with the value: through fields, string
// concatenation, comparisons and function calls, the result takes the highest
// label of its inputs (taint propagation). Branching or looping on a labelled
// value is also refused, because what is rendered would reveal something about
// it (an implicit flow).
//
// Sinks — a view, an attribute, a log line — accept only Public. The only way
// across the boundary is explicit: masked(value, n) or reveal(value). Every
// such crossing is recorded, and `complex audit` prints the list. If the
// checker reports an error, nothing runs.

use std::collections::HashMap;
use std::fmt;

use crate::ast::*;
use crate::error::{did_you_mean, CompileError, Pos};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Label {
    Public,
    Personal,
    Secret,
}

impl Label {
    pub fn name(self) -> &'static str {
        match self {
            Label::Public => "public",
            Label::Personal => "personal",
            Label::Secret => "secret",
        }
    }

    fn join(self, other: Label) -> Label {
        self.max(other)
    }

    fn parse(s: &str) -> Option<Label> {
        match s {
            "public" => Some(Label::Public),
            "personal" => Some(Label::Personal),
            "secret" => Some(Label::Secret),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Text,
    Int,
    Bool,
    List(Box<Ty>),
    Record(Vec<(String, Ty)>),
    /// One record of a model.
    Model(String),
    /// The model itself, as in `Citizen.all` or `Citizen[id]`.
    ModelRef(String),
    /// A value with a label above Public. The inner type is never itself Labeled.
    Labeled(Box<Ty>, Label),
    /// Used internally after an error, so one mistake does not cause ten.
    Unknown,
}

impl fmt::Display for Ty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Ty::Text => write!(f, "Text"),
            Ty::Int => write!(f, "Int"),
            Ty::Bool => write!(f, "Bool"),
            Ty::List(t) => write!(f, "[{t}]"),
            Ty::Record(fields) => {
                let parts: Vec<String> = fields.iter().map(|(n, t)| format!("{n}: {t}")).collect();
                write!(f, "{{ {} }}", parts.join(", "))
            }
            Ty::Model(m) => write!(f, "{m}"),
            Ty::ModelRef(m) => write!(f, "model {m}"),
            Ty::Labeled(t, Label::Secret) => write!(f, "Secret<{t}>"),
            Ty::Labeled(t, Label::Personal) => write!(f, "Personal<{t}>"),
            Ty::Labeled(t, Label::Public) => write!(f, "{t}"),
            Ty::Unknown => write!(f, "?"),
        }
    }
}

impl Ty {
    /// Splits a type into its plain shape and its label.
    fn strip(&self) -> (Ty, Label) {
        match self {
            Ty::Labeled(inner, l) => (*inner.clone(), *l),
            t => (t.clone(), Label::Public),
        }
    }

    pub fn label(&self) -> Label {
        self.strip().1
    }

    /// Raises the label of a type to at least `label`.
    fn raise(self, label: Label) -> Ty {
        let (base, own) = self.strip();
        let l = own.join(label);
        if l == Label::Public {
            base
        } else {
            Ty::Labeled(Box::new(base), l)
        }
    }
}

/// Structural type equality; labels must match too, field order does not.
fn same_type(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        (Ty::List(x), Ty::List(y)) => same_type(x, y),
        (Ty::Labeled(x, lx), Ty::Labeled(y, ly)) => lx == ly && same_type(x, y),
        (Ty::Record(x), Ty::Record(y)) => {
            x.len() == y.len() && x.iter().all(|(n, t)| y.iter().any(|(m, u)| n == m && same_type(t, u)))
        }
        _ => a == b,
    }
}

// ---- audit trail -----------------------------------------------------------

/// One place where a labelled value crosses into public, on purpose.
#[derive(Debug, Clone)]
pub struct Declassification {
    pub file: String,
    pub pos: Pos,
    pub how: String,
    pub from: Label,
    pub line: String,
}

#[derive(Debug, Clone)]
pub struct LabeledField {
    pub model: String,
    pub field: String,
    pub label: Label,
}

/// What `complex audit` prints: the facts the checker proved.
#[derive(Debug, Default, Clone)]
pub struct Audit {
    pub labeled_fields: Vec<LabeledField>,
    pub declassifications: Vec<Declassification>,
    pub sinks_view: usize,
    pub sinks_attr: usize,
    pub sinks_branch: usize,
    pub sinks_loop: usize,
    pub sinks_log: usize,
}

pub fn check(program: &Program) -> Result<Audit, Vec<CompileError>> {
    let mut c = Checker {
        program,
        errors: Vec::new(),
        scopes: vec![HashMap::new()],
        file: String::new(),
        in_component: None,
        audit: Audit::default(),
    };
    c.run();
    if c.errors.is_empty() {
        Ok(c.audit)
    } else {
        Err(c.errors)
    }
}

struct Checker<'a> {
    program: &'a Program,
    errors: Vec<CompileError>,
    scopes: Vec<HashMap<String, Ty>>,
    file: String,
    in_component: Option<String>,
    audit: Audit,
}

impl<'a> Checker<'a> {
    // ---- helpers ------------------------------------------------------------

    fn source(&self) -> &str {
        self.program.files.iter().find(|f| f.name == self.file).map(|f| f.source.as_str()).unwrap_or("")
    }

    fn error(&mut self, pos: Pos, msg: impl Into<String>, hint: Option<String>) {
        let mut e = CompileError::new(&self.file, self.source(), pos, msg);
        e.hint = hint;
        self.errors.push(e);
    }

    fn lookup(&self, name: &str) -> Option<&Ty> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    fn visible_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.scopes.iter().flat_map(|s| s.keys().cloned()).collect();
        names.extend(self.program.models.iter().map(|m| m.name.clone()));
        names
    }

    fn bind(&mut self, name: &str, ty: Ty) {
        self.scopes.last_mut().expect("scope").insert(name.to_string(), ty);
    }

    fn component(&self, name: &str) -> Option<&'a Component> {
        self.program.components.iter().find(|c| c.name == name)
    }

    fn model(&self, name: &str) -> Option<&'a Model> {
        self.program.models.iter().find(|m| m.name == name)
    }

    fn line_text(&self, pos: Pos) -> String {
        self.source().lines().nth(pos.line.saturating_sub(1)).unwrap_or("").trim().to_string()
    }

    // ---- program ------------------------------------------------------------

    fn run(&mut self) {
        let p = self.program;
        self.check_duplicates();
        self.check_models();
        self.check_seeds();

        for l in &p.lets {
            self.file = l.file.clone();
            let ty = self.expr(&l.value);
            self.bind(&l.name, ty);
        }

        for comp in &p.components {
            self.file = comp.file.clone();
            self.scopes.push(HashMap::new());
            for param in &comp.params {
                let ty = self.resolve_type(&param.ty, param.pos);
                self.bind(&param.name, ty);
            }
            self.in_component = Some(comp.name.clone());
            self.nodes(&comp.body);
            self.in_component = None;
            self.scopes.pop();
        }

        for page in &p.pages {
            self.file = page.file.clone();
            self.scopes.push(HashMap::new());
            let url_params: Vec<&str> = page.pattern.split('/').filter_map(|s| s.strip_prefix(':')).collect();
            for param in &page.params {
                if !url_params.contains(&param.name.as_str()) {
                    self.error(param.pos, format!("parameter '{}' is not in the page address", param.name), Some(format!("add :{} to the address, like \"/x/:{}\"", param.name, param.name)));
                }
                let ty = self.resolve_type(&param.ty, param.pos);
                if !matches!(ty, Ty::Text | Ty::Int | Ty::Unknown) {
                    self.error(param.pos, format!("a URL parameter can be Text or Int, not {ty}"), None);
                }
                self.bind(&param.name, ty);
            }
            for name in url_params {
                if self.lookup(name).is_none() {
                    self.bind(name, Ty::Text);
                }
            }
            for stmt in &page.stmts {
                match stmt {
                    Stmt::Let(l) => {
                        let ty = self.expr(&l.value);
                        self.bind(&l.name, ty);
                    }
                    Stmt::Log { pos, value } => {
                        self.audit.sinks_log += 1;
                        let ty = self.expr(value);
                        self.sink(&ty, *pos, "written to the log");
                    }
                }
            }
            self.nodes(&page.view);
            self.scopes.pop();
        }
    }

    fn check_duplicates(&mut self) {
        let p = self.program;
        let mut seen: HashMap<String, ()> = HashMap::new();
        for l in &p.lets {
            if seen.insert(l.name.clone(), ()).is_some() {
                self.file = l.file.clone();
                self.error(l.pos, format!("'{}' is declared twice", l.name), Some("rename one of them".into()));
            }
        }
        let mut comps: HashMap<String, ()> = HashMap::new();
        for c in &p.components {
            self.file = c.file.clone();
            if comps.insert(c.name.clone(), ()).is_some() {
                self.error(c.pos, format!("component '{}' is declared twice", c.name), None);
            }
            if HTML_TAGS.contains(&c.name.as_str()) {
                self.error(c.pos, format!("component '{}' has the same name as an HTML element", c.name), Some("give it its own name".into()));
            }
        }
        let mut models: HashMap<String, ()> = HashMap::new();
        for m in &p.models {
            self.file = m.file.clone();
            if models.insert(m.name.clone(), ()).is_some() {
                self.error(m.pos, format!("model '{}' is declared twice", m.name), None);
            }
        }
        let mut pages: HashMap<String, ()> = HashMap::new();
        for pg in &p.pages {
            if pages.insert(pg.pattern.clone(), ()).is_some() {
                self.file = pg.file.clone();
                self.error(pg.pos, format!("page \"{}\" is declared twice", pg.pattern), None);
            }
        }
    }

    fn check_models(&mut self) {
        for m in &self.program.models {
            self.file = m.file.clone();
            let mut seen: HashMap<String, ()> = HashMap::new();
            for f in &m.fields {
                if seen.insert(f.name.clone(), ()).is_some() {
                    self.error(f.pos, format!("field '{}' appears twice in {}", f.name, m.name), None);
                }
                let ty = self.resolve_type(&f.ty, f.pos);
                if matches!(ty, Ty::Labeled(..)) {
                    self.error(f.pos, "a model field is labelled with @secret or @personal, not with Secret<…>".to_string(), Some(format!("write `{}: Text @secret`", f.name)));
                }
                if let Some((label, lpos)) = &f.label {
                    match Label::parse(label) {
                        Some(Label::Public) => self.error(*lpos, "@public is the default; leave the label out".to_string(), None),
                        Some(l) => self.audit.labeled_fields.push(LabeledField { model: m.name.clone(), field: f.name.clone(), label: l }),
                        None => {
                            let hint = did_you_mean(label, ["secret", "personal"]).map(|s| format!("did you mean '@{s}'?"));
                            self.error(*lpos, format!("unknown label '@{label}'"), hint.or(Some("labels are @secret and @personal".into())));
                        }
                    }
                }
            }
        }
    }

    fn check_seeds(&mut self) {
        for s in &self.program.seeds {
            self.file = s.file.clone();
            let Some(model) = self.model(&s.model) else {
                let hint = did_you_mean(&s.model, self.program.models.iter().map(|m| m.name.as_str())).map(|x| format!("did you mean '{x}'?"));
                self.error(s.pos, format!("unknown model '{}'", s.model), hint);
                continue;
            };
            let Expr::List { items, .. } = &s.value else {
                self.error(s.value.pos(), "seed data is a list of records: seed Citizen [ { ... } ]", None);
                continue;
            };
            for item in items {
                let Expr::Record { fields, pos } = item else {
                    self.error(item.pos(), format!("each seed item is a {} record: {{ field: value, ... }}", model.name), None);
                    continue;
                };
                for mf in &model.fields {
                    match fields.iter().find(|f| f.name == mf.name) {
                        None => self.error(*pos, format!("seed record is missing the field '{}'", mf.name), None),
                        Some(f) => {
                            let expected = self.resolve_type(&mf.ty, mf.pos);
                            let actual = self.expr(&f.value);
                            if !same_type(&actual, &expected) {
                                self.error(f.pos, format!("'{}' is {} but this value is {}", mf.name, expected, actual), None);
                            }
                        }
                    }
                }
                for f in fields {
                    if !model.fields.iter().any(|mf| mf.name == f.name) {
                        let hint = did_you_mean(&f.name, model.fields.iter().map(|mf| mf.name.as_str())).map(|x| format!("did you mean '{x}'?"));
                        self.error(f.pos, format!("{} has no field '{}'", model.name, f.name), hint);
                    }
                }
            }
        }
    }

    fn resolve_type(&mut self, t: &TypeExpr, pos: Pos) -> Ty {
        match t {
            TypeExpr::Named(n) => match n.as_str() {
                "Text" => Ty::Text,
                "Int" => Ty::Int,
                "Bool" => Ty::Bool,
                other => {
                    if self.model(other).is_some() {
                        return Ty::Model(other.to_string());
                    }
                    let known: Vec<&str> = ["Text", "Int", "Bool"].into_iter().chain(self.program.models.iter().map(|m| m.name.as_str())).collect();
                    let hint = did_you_mean(other, known).map(|s| format!("did you mean '{s}'?"));
                    self.error(pos, format!("unknown type '{other}'"), hint);
                    Ty::Unknown
                }
            },
            TypeExpr::List(inner) => Ty::List(Box::new(self.resolve_type(inner, pos))),
            TypeExpr::Generic(name, inner) => {
                let inner_ty = self.resolve_type(inner, pos);
                match name.as_str() {
                    "Secret" => inner_ty.raise(Label::Secret),
                    "Personal" => inner_ty.raise(Label::Personal),
                    _ => {
                        self.error(pos, format!("unknown generic type '{name}'"), Some("the generic types are Secret<T> and Personal<T>".into()));
                        Ty::Unknown
                    }
                }
            }
        }
    }

    // ---- view nodes ---------------------------------------------------------

    fn nodes(&mut self, nodes: &[Node]) {
        for n in nodes {
            self.node(n);
        }
    }

    fn node(&mut self, node: &Node) {
        match node {
            Node::Text(e) => {
                self.audit.sinks_view += 1;
                let ty = self.expr(e);
                self.sink(&ty, e.pos(), "shown in a view");
            }
            Node::Element { pos, name, args, children, has_body } => {
                if let Some(comp) = self.component(name) {
                    self.component_call(comp, *pos, args, *has_body);
                    self.nodes(children);
                } else if HTML_TAGS.contains(&name.as_str()) {
                    self.html_element(*pos, name, args, *has_body);
                    self.nodes(children);
                } else {
                    let candidates: Vec<&str> = HTML_TAGS.iter().copied().chain(self.program.components.iter().map(|c| c.name.as_str())).collect();
                    let hint = did_you_mean(name, candidates).map(|s| format!("did you mean '{s}'?"));
                    self.error(*pos, format!("unknown element or component '{name}'"), hint);
                }
            }
            Node::For { pos, var, list, body } => {
                self.audit.sinks_loop += 1;
                let ty = self.expr(list);
                let (base, label) = ty.strip();
                let item = match base {
                    Ty::List(item) if label == Label::Public => *item,
                    Ty::List(item) => {
                        self.error(*pos, format!("cannot loop over {} data", label.name()), Some("the number of items rendered would reveal something about it (an implicit flow)".into()));
                        *item
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.error(list.pos(), format!("'for' needs a list, but this is {other}"), None);
                        Ty::Unknown
                    }
                };
                self.scopes.push(HashMap::new());
                self.bind(var, item);
                self.nodes(body);
                self.scopes.pop();
            }
            Node::If { pos, cond, then, otherwise } => {
                self.audit.sinks_branch += 1;
                let ty = self.expr(cond);
                let (base, label) = ty.strip();
                match base {
                    Ty::Bool if label != Label::Public => self.error(
                        *pos,
                        format!("cannot branch on {} data", label.name()),
                        Some("showing different content depending on it tells the visitor something about it (an implicit flow)".into()),
                    ),
                    Ty::Bool | Ty::Unknown => {}
                    other => self.error(cond.pos(), format!("'if' needs a Bool, but this is {other}"), Some("compare it, like `x == \"\"`".into())),
                }
                self.nodes(then);
                self.nodes(otherwise);
            }
            Node::Children { pos } => {
                if self.in_component.is_none() {
                    self.error(*pos, "'children' can only be used inside a component", None);
                }
            }
        }
    }

    /// A sink accepts only public, renderable values.
    fn sink(&mut self, ty: &Ty, pos: Pos, place: &str) {
        let (base, label) = ty.strip();
        if label != Label::Public {
            let hint = match label {
                Label::Secret => "reveal part of it on purpose with masked(value, 3); every masked() call is listed by `complex audit`",
                _ => "show it on purpose with reveal(value); every reveal() call is listed by `complex audit`",
            };
            self.error(pos, format!("this value is {ty} and cannot be {place}"), Some(hint.into()));
            return;
        }
        match base {
            Ty::Text | Ty::Int | Ty::Bool | Ty::Unknown => {}
            Ty::List(_) => self.error(pos, format!("a list cannot be {place}"), Some("use `for item in list { ... }`".into())),
            Ty::Record(fields) => {
                let first = fields.first().map(|(n, _)| n.as_str()).unwrap_or("field");
                self.error(pos, format!("a record cannot be {place}"), Some(format!("show one of its fields, like value.{first}")))
            }
            Ty::Model(m) => self.error(pos, format!("a {m} record cannot be {place}"), Some("show one of its fields".into())),
            Ty::ModelRef(m) => self.error(pos, format!("the model {m} cannot be {place}"), Some(format!("use {m}.all or {m}[id]"))),
            Ty::Labeled(..) => unreachable!("stripped above"),
        }
    }

    fn html_element(&mut self, pos: Pos, name: &str, args: &[Arg], has_body: bool) {
        if has_body && VOID_TAGS.contains(&name) {
            self.error(pos, format!("<{name}> cannot have children"), Some("remove the { ... } block".into()));
        }
        for arg in args {
            self.audit.sinks_attr += 1;
            let ty = self.expr(&arg.value);
            self.sink(&ty, arg.value.pos(), &format!("placed in the '{}' attribute", arg.name));
            if matches!(arg.name.as_str(), "href" | "src" | "action") {
                if let Expr::Str { value, pos } = &arg.value {
                    if value.trim().to_ascii_lowercase().starts_with("javascript:") {
                        self.error(*pos, "javascript: addresses are not allowed", Some("script URLs are an XSS risk".into()));
                    }
                }
            }
        }
    }

    fn component_call(&mut self, comp: &Component, pos: Pos, args: &[Arg], has_body: bool) {
        let comp_file = comp.file.clone();
        let call_file = self.file.clone();
        for param in &comp.params {
            self.file = comp_file.clone();
            let expected = self.resolve_type(&param.ty, param.pos);
            self.file = call_file.clone();
            match args.iter().find(|a| a.name == param.name) {
                None => self.error(pos, format!("'{}' needs the argument '{}: {}'", comp.name, param.name, expected), None),
                Some(arg) => {
                    let actual = self.expr(&arg.value);
                    if !same_type(&actual, &expected) {
                        let hint = if actual.label() > expected.label() {
                            Some(format!("{} data does not become public by passing it on; declare the parameter as {} or declassify first", actual.label().name(), actual))
                        } else {
                            None
                        };
                        self.error(arg.pos, format!("'{}' expects {} but got {}", param.name, expected, actual), hint);
                    }
                }
            }
        }
        for arg in args {
            if !comp.params.iter().any(|p| p.name == arg.name) {
                let hint = did_you_mean(&arg.name, comp.params.iter().map(|p| p.name.as_str())).map(|s| format!("did you mean '{s}'?"));
                self.error(arg.pos, format!("'{}' has no parameter '{}'", comp.name, arg.name), hint);
            }
        }
        if has_body && !uses_children(&comp.body) {
            self.error(pos, format!("'{}' does not show children, so the {{ ... }} block would be lost", comp.name), Some("add `children` inside the component, or remove the block".into()));
        }
    }

    // ---- expressions --------------------------------------------------------

    fn expr(&mut self, e: &Expr) -> Ty {
        match e {
            Expr::Str { .. } => Ty::Text,
            Expr::Int { .. } => Ty::Int,
            Expr::Bool { .. } => Ty::Bool,
            Expr::List { items, .. } => {
                let mut item_ty: Option<Ty> = None;
                for item in items {
                    let t = self.expr(item);
                    match &item_ty {
                        None => item_ty = Some(t),
                        Some(first) if !same_type(first, &t) => {
                            let msg = format!("list items must all have the same type: the first is {first}, this one is {t}");
                            self.error(item.pos(), msg, None);
                        }
                        _ => {}
                    }
                }
                Ty::List(Box::new(item_ty.unwrap_or(Ty::Unknown)))
            }
            Expr::Record { fields, .. } => {
                let mut out: Vec<(String, Ty)> = Vec::new();
                for f in fields {
                    if out.iter().any(|(n, _)| n == &f.name) {
                        self.error(f.pos, format!("field '{}' appears twice", f.name), None);
                        continue;
                    }
                    let t = self.expr(&f.value);
                    out.push((f.name.clone(), t));
                }
                Ty::Record(out)
            }
            Expr::Name { pos, name } => {
                if let Some(t) = self.lookup(name) {
                    return t.clone();
                }
                if self.model(name).is_some() {
                    return Ty::ModelRef(name.clone());
                }
                let names = self.visible_names();
                let hint = did_you_mean(name, names.iter().map(|s| s.as_str())).map(|s| format!("did you mean '{s}'?"));
                self.error(*pos, format!("'{name}' is not defined"), hint);
                Ty::Unknown
            }
            Expr::Field { pos, target, name } => {
                let (base, label) = self.expr(target).strip();
                match base {
                    Ty::Record(fields) => match fields.iter().find(|(n, _)| n == name) {
                        Some((_, t)) => t.clone().raise(label),
                        None => {
                            let hint = did_you_mean(name, fields.iter().map(|(n, _)| n.as_str())).map(|s| format!("did you mean '{s}'?"));
                            self.error(*pos, format!("this record has no field '{name}'"), hint);
                            Ty::Unknown
                        }
                    },
                    Ty::Model(m) => {
                        let model = self.model(&m).expect("model exists");
                        match model.fields.iter().find(|f| &f.name == name) {
                            Some(f) => {
                                let fl = f.label.as_ref().and_then(|(l, _)| Label::parse(l)).unwrap_or(Label::Public);
                                let t = self.resolve_type(&f.ty, f.pos);
                                t.raise(fl).raise(label)
                            }
                            None => {
                                let hint = did_you_mean(name, model.fields.iter().map(|f| f.name.as_str())).map(|s| format!("did you mean '{s}'?"));
                                self.error(*pos, format!("{m} has no field '{name}'"), hint);
                                Ty::Unknown
                            }
                        }
                    }
                    Ty::ModelRef(m) => {
                        if name == "all" {
                            Ty::List(Box::new(Ty::Model(m)))
                        } else {
                            self.error(*pos, format!("the model {m} has no '{name}'"), Some(format!("use {m}.all for every record, or {m}[id] for one")));
                            Ty::Unknown
                        }
                    }
                    Ty::Unknown => Ty::Unknown,
                    Ty::List(_) => {
                        self.error(*pos, format!("a list has no field '{name}'"), Some("use `for item in list { item.field }`".into()));
                        Ty::Unknown
                    }
                    other => {
                        self.error(*pos, format!("{other} has no field '{name}'"), None);
                        Ty::Unknown
                    }
                }
            }
            Expr::Index { pos, target, index } => {
                let t = self.expr(target);
                let i = self.expr(index);
                match t {
                    Ty::ModelRef(m) => {
                        let (ib, il) = i.strip();
                        if il != Label::Public {
                            self.error(index.pos(), format!("cannot look up a record by {} data", il.name()), Some("which record is shown would reveal the value (an implicit flow)".into()));
                        } else if !matches!(ib, Ty::Int | Ty::Unknown) {
                            self.error(index.pos(), format!("{m}[…] needs an Int id, but this is {ib}"), Some("declare the page parameter as Int: page \"/x/:id\" (id: Int)".into()));
                        }
                        Ty::Model(m)
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        self.error(*pos, format!("{other} cannot be indexed"), Some("only models can be indexed, like Citizen[id]".into()));
                        Ty::Unknown
                    }
                }
            }
            Expr::Binary { pos, op, left, right } => {
                let (l, ll) = self.expr(left).strip();
                let (r, rl) = self.expr(right).strip();
                let result = match op {
                    BinOp::Add => match (&l, &r) {
                        (Ty::Unknown, _) | (_, Ty::Unknown) => Ty::Unknown,
                        (Ty::Int, Ty::Int) => Ty::Int,
                        (Ty::Text, Ty::Text) | (Ty::Text, Ty::Int) | (Ty::Int, Ty::Text) => Ty::Text,
                        _ => {
                            self.error(*pos, format!("cannot add {l} and {r}"), None);
                            Ty::Unknown
                        }
                    },
                    BinOp::Eq | BinOp::NotEq => {
                        if !same_type(&l, &r) {
                            self.error(*pos, format!("cannot compare {l} with {r}"), None);
                        }
                        Ty::Bool
                    }
                };
                // Anything computed from labelled data carries the highest label (taint propagation).
                result.raise(ll.join(rl))
            }
            Expr::Call { pos, name, args } => self.call(*pos, name, args),
        }
    }

    fn call(&mut self, pos: Pos, name: &str, args: &[Expr]) -> Ty {
        let tys: Vec<Ty> = args.iter().map(|a| self.expr(a)).collect();
        let arity = |n: usize, this: &mut Self| -> bool {
            if tys.len() != n {
                this.error(pos, format!("{name}() takes {n} argument{}, but got {}", if n == 1 { "" } else { "s" }, tys.len()), None);
                false
            } else {
                true
            }
        };
        match name {
            "now" => {
                arity(0, self);
                Ty::Text
            }
            "year" => {
                arity(0, self);
                Ty::Int
            }
            "len" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                let (base, label) = tys[0].strip();
                if !matches!(base, Ty::List(_) | Ty::Text | Ty::Unknown) {
                    self.error(args[0].pos(), format!("len() needs a list or text, but this is {base}"), None);
                }
                Ty::Int.raise(label)
            }
            "upper" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                let (base, label) = tys[0].strip();
                if !matches!(base, Ty::Text | Ty::Unknown) {
                    self.error(args[0].pos(), format!("upper() needs Text, but this is {base}"), None);
                }
                Ty::Text.raise(label)
            }
            "secret" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                tys[0].clone().raise(Label::Secret)
            }
            "masked" => {
                if !arity(2, self) {
                    return Ty::Unknown;
                }
                let (base, label) = tys[0].strip();
                if label == Label::Public && base != Ty::Unknown {
                    self.error(args[0].pos(), format!("masked() is for labelled data, and this {base} is already public"), Some("remove masked()".into()));
                } else if !matches!(base, Ty::Text | Ty::Unknown) {
                    self.error(args[0].pos(), format!("masked() works on text, but this is {base}"), None);
                }
                if !matches!(tys[1], Ty::Int | Ty::Unknown) {
                    self.error(args[1].pos(), "masked() needs the number of characters to keep, like masked(value, 3)", None);
                }
                if label != Label::Public {
                    self.declassify(pos, "masked", label);
                }
                Ty::Text
            }
            "reveal" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                let (base, label) = tys[0].strip();
                match label {
                    Label::Public => {
                        if base != Ty::Unknown {
                            self.error(args[0].pos(), format!("reveal() is for labelled data, and this {base} is already public"), Some("remove reveal()".into()));
                        }
                    }
                    Label::Secret => self.error(
                        args[0].pos(),
                        "reveal() cannot show secret data in full".to_string(),
                        Some("secrets only leave through masked(value, n); use @personal for data that may be shown on purpose".into()),
                    ),
                    Label::Personal => self.declassify(pos, "reveal", label),
                }
                base
            }
            _ => {
                let hint = if self.component(name).is_some() {
                    Some(format!("'{name}' is a component; use it inside a view, like {name}(...)"))
                } else {
                    did_you_mean(name, BUILTINS).map(|s| format!("did you mean '{s}'?"))
                };
                self.error(pos, format!("unknown function '{name}'"), hint);
                Ty::Unknown
            }
        }
    }

    fn declassify(&mut self, pos: Pos, how: &str, from: Label) {
        self.audit.declassifications.push(Declassification {
            file: self.file.clone(),
            pos,
            how: how.to_string(),
            from,
            line: self.line_text(pos),
        });
    }
}

fn uses_children(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Children { .. } => true,
        Node::Element { children, .. } => uses_children(children),
        Node::For { body, .. } => uses_children(body),
        Node::If { then, otherwise, .. } => uses_children(then) || uses_children(otherwise),
        Node::Text(_) => false,
    })
}

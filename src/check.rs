// RobertBlok.com — Fullstack development
//
// The checker runs before anything is served. It resolves every name,
// infers every type, and enforces information flow control:
// a value of type Secret<T> can never reach a view, an attribute,
// a branch condition or a loop, unless it is explicitly declassified
// with masked(). If the checker reports an error, nothing runs.

use std::collections::HashMap;
use std::fmt;

use crate::ast::*;
use crate::error::{did_you_mean, CompileError, Pos};

#[derive(Debug, Clone, PartialEq)]
pub enum Ty {
    Text,
    Int,
    Bool,
    List(Box<Ty>),
    Record(Vec<(String, Ty)>),
    Secret(Box<Ty>),
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
            Ty::Secret(t) => write!(f, "Secret<{t}>"),
            Ty::Unknown => write!(f, "?"),
        }
    }
}

impl Ty {
    fn is_secret(&self) -> bool {
        matches!(self, Ty::Secret(_))
    }

    /// Removes the Secret wrapper, reporting whether there was one.
    fn unwrap_secret(&self) -> (Ty, bool) {
        match self {
            Ty::Secret(inner) => (*inner.clone(), true),
            t => (t.clone(), false),
        }
    }

    fn secret_if(self, secret: bool) -> Ty {
        if secret && !self.is_secret() {
            Ty::Secret(Box::new(self))
        } else {
            self
        }
    }
}

/// Structural type equality; field order in records does not matter.
fn same_type(a: &Ty, b: &Ty) -> bool {
    match (a, b) {
        (Ty::Unknown, _) | (_, Ty::Unknown) => true,
        (Ty::List(x), Ty::List(y)) | (Ty::Secret(x), Ty::Secret(y)) => same_type(x, y),
        (Ty::Record(x), Ty::Record(y)) => {
            x.len() == y.len()
                && x.iter().all(|(n, t)| y.iter().any(|(m, u)| n == m && same_type(t, u)))
        }
        _ => a == b,
    }
}

pub fn check(program: &Program) -> Result<(), Vec<CompileError>> {
    let mut c = Checker {
        program,
        errors: Vec::new(),
        scopes: vec![HashMap::new()],
        file: String::new(),
        in_component: None,
    };
    c.run();
    if c.errors.is_empty() {
        Ok(())
    } else {
        Err(c.errors)
    }
}

struct Checker<'a> {
    program: &'a Program,
    errors: Vec<CompileError>,
    scopes: Vec<HashMap<String, Ty>>,
    file: String,
    /// Name of the component being checked, if any.
    in_component: Option<String>,
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
        self.scopes.iter().flat_map(|s| s.keys().cloned()).collect()
    }

    fn bind(&mut self, name: &str, ty: Ty) {
        self.scopes.last_mut().expect("scope").insert(name.to_string(), ty);
    }

    fn component(&self, name: &str) -> Option<&'a Component> {
        self.program.components.iter().find(|c| c.name == name)
    }

    // ---- program ------------------------------------------------------------

    fn run(&mut self) {
        let p = self.program;
        self.check_duplicates();

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
            for seg in page.pattern.split('/') {
                if let Some(param) = seg.strip_prefix(':') {
                    self.bind(param, Ty::Text);
                }
            }
            for l in &page.lets {
                let ty = self.expr(&l.value);
                self.bind(&l.name, ty);
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
        let mut pages: HashMap<String, ()> = HashMap::new();
        for pg in &p.pages {
            if pages.insert(pg.pattern.clone(), ()).is_some() {
                self.file = pg.file.clone();
                self.error(pg.pos, format!("page \"{}\" is declared twice", pg.pattern), None);
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
                    let hint = did_you_mean(other, ["Text", "Int", "Bool"]).map(|s| format!("did you mean '{s}'?"));
                    self.error(pos, format!("unknown type '{other}'"), hint);
                    Ty::Unknown
                }
            },
            TypeExpr::List(inner) => Ty::List(Box::new(self.resolve_type(inner, pos))),
            TypeExpr::Generic(name, inner) if name == "Secret" => Ty::Secret(Box::new(self.resolve_type(inner, pos))),
            TypeExpr::Generic(name, _) => {
                self.error(pos, format!("unknown generic type '{name}'"), Some("the only generic type in v0.1 is Secret<T>".into()));
                Ty::Unknown
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
                let ty = self.expr(e);
                self.renderable(&ty, e.pos(), "shown in a view");
            }
            Node::Element { pos, name, args, children, has_body } => {
                if let Some(comp) = self.component(name) {
                    self.component_call(comp, *pos, args, *has_body);
                    self.nodes(children);
                } else if HTML_TAGS.contains(&name.as_str()) {
                    self.html_element(*pos, name, args, *has_body);
                    self.nodes(children);
                } else {
                    let candidates: Vec<&str> = HTML_TAGS
                        .iter()
                        .copied()
                        .chain(self.program.components.iter().map(|c| c.name.as_str()))
                        .collect();
                    let hint = did_you_mean(name, candidates).map(|s| format!("did you mean '{s}'?"));
                    self.error(*pos, format!("unknown element or component '{name}'"), hint);
                }
            }
            Node::For { pos, var, list, body } => {
                let ty = self.expr(list);
                let item = match ty {
                    Ty::List(item) => *item,
                    Ty::Unknown => Ty::Unknown,
                    Ty::Secret(_) => {
                        self.error(
                            *pos,
                            "cannot loop over a secret".to_string(),
                            Some("the number of items rendered would reveal its length (an implicit flow)".into()),
                        );
                        Ty::Unknown
                    }
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
                let ty = self.expr(cond);
                match ty {
                    Ty::Bool | Ty::Unknown => {}
                    Ty::Secret(_) => self.error(
                        *pos,
                        "cannot branch on a secret".to_string(),
                        Some("showing different content depending on a secret tells the visitor something about it (an implicit flow)".into()),
                    ),
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

    /// Reports an error when a value of this type may not be turned into HTML.
    fn renderable(&mut self, ty: &Ty, pos: Pos, place: &str) {
        match ty {
            Ty::Text | Ty::Int | Ty::Bool | Ty::Unknown => {}
            Ty::Secret(inner) => self.error(
                pos,
                format!("this value is Secret<{inner}> and cannot be {place}"),
                Some("reveal part of it on purpose with masked(value, 3); every masked() call is a line an auditor can find".into()),
            ),
            Ty::List(_) => self.error(pos, format!("a list cannot be {place}"), Some("use `for item in list { ... }`".into())),
            Ty::Record(fields) => {
                let first = fields.first().map(|(n, _)| n.as_str()).unwrap_or("field");
                self.error(pos, format!("a record cannot be {place}"), Some(format!("show one of its fields, like value.{first}")))
            }
        }
    }

    fn html_element(&mut self, pos: Pos, name: &str, args: &[Arg], has_body: bool) {
        if has_body && VOID_TAGS.contains(&name) {
            self.error(pos, format!("<{name}> cannot have children"), Some("remove the { ... } block".into()));
        }
        for arg in args {
            let ty = self.expr(&arg.value);
            self.renderable(&ty, arg.value.pos(), &format!("placed in the '{}' attribute", arg.name));
            if matches!(arg.name.as_str(), "href" | "src" | "action") {
                if let Expr::Str { value, pos } = &arg.value {
                    if value.trim().to_ascii_lowercase().starts_with("javascript:") {
                        self.error(*pos, "javascript: addresses are not allowed", Some("use an action instead; script URLs are an XSS risk".into()));
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
                        let hint = if actual.is_secret() && !expected.is_secret() {
                            Some("a secret does not become public by passing it on; use masked(...)".to_string())
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
            Expr::Name { pos, name } => match self.lookup(name) {
                Some(t) => t.clone(),
                None => {
                    let names = self.visible_names();
                    let hint = did_you_mean(name, names.iter().map(|s| s.as_str())).map(|s| format!("did you mean '{s}'?"));
                    self.error(*pos, format!("'{name}' is not defined"), hint);
                    Ty::Unknown
                }
            },
            Expr::Field { pos, target, name } => {
                let (base, secret) = self.expr(target).unwrap_secret();
                match base {
                    Ty::Record(fields) => match fields.iter().find(|(n, _)| n == name) {
                        Some((_, t)) => t.clone().secret_if(secret),
                        None => {
                            let hint = did_you_mean(name, fields.iter().map(|(n, _)| n.as_str())).map(|s| format!("did you mean '{s}'?"));
                            self.error(*pos, format!("this record has no field '{name}'"), hint);
                            Ty::Unknown
                        }
                    },
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
            Expr::Binary { pos, op, left, right } => {
                let (l, ls) = self.expr(left).unwrap_secret();
                let (r, rs) = self.expr(right).unwrap_secret();
                let secret = ls || rs;
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
                // Anything computed from a secret is itself secret (taint propagation).
                result.secret_if(secret)
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
                let (base, secret) = tys[0].unwrap_secret();
                if !matches!(base, Ty::List(_) | Ty::Text | Ty::Unknown) {
                    self.error(args[0].pos(), format!("len() needs a list or text, but this is {base}"), None);
                }
                Ty::Int.secret_if(secret)
            }
            "upper" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                let (base, secret) = tys[0].unwrap_secret();
                if !matches!(base, Ty::Text | Ty::Unknown) {
                    self.error(args[0].pos(), format!("upper() needs Text, but this is {base}"), None);
                }
                Ty::Text.secret_if(secret)
            }
            "secret" => {
                if !arity(1, self) {
                    return Ty::Unknown;
                }
                tys[0].clone().secret_if(true)
            }
            "masked" => {
                if !arity(2, self) {
                    return Ty::Unknown;
                }
                match &tys[0] {
                    Ty::Secret(inner) if matches!(**inner, Ty::Text | Ty::Unknown) => {}
                    Ty::Unknown => {}
                    Ty::Secret(inner) => self.error(args[0].pos(), format!("masked() works on Secret<Text>, but this is Secret<{inner}>"), None),
                    other => self.error(args[0].pos(), format!("masked() is only for secrets, and this {other} is already public"), Some("remove masked()".into())),
                }
                if !matches!(tys[1], Ty::Int | Ty::Unknown) {
                    self.error(args[1].pos(), "masked() needs the number of characters to keep, like masked(value, 3)", None);
                }
                Ty::Text
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

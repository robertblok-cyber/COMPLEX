// RobertBlok.com — Fullstack development
//
// A hand-written recursive descent parser: one function per grammar rule.
//
//   program   = { let | model | seed | component | page }
//   let       = "let" NAME "=" expr
//   model     = "model" NAME "{" { NAME ":" type [ "@" NAME ] [ "," ] } "}"
//   seed      = "seed" NAME expr
//   component = "component" NAME "(" [ param { "," param } ] ")" nodes
//   page      = "page" TEXT [ "(" params ")" ] "{" { let | "log" expr } "view" nodes "}"
//   nodes     = "{" { node [ "," ] } "}"
//   node      = for | if | element | expr
//   element   = NAME [ "(" [ arg { "," arg } ] ")" ] [ nodes ]
//   expr      = sum { ("==" | "!=") sum }
//   sum       = postfix { "+" postfix }
//   postfix   = primary { "." NAME | "[" expr "]" }
//   primary   = TEXT | INT | "true" | "false" | list | record | call | NAME | "(" expr ")"

use crate::ast::*;
use crate::error::{CompileError, Pos};
use crate::lexer::{kw_text, Kw, Tok, Token};

pub struct Parser<'a> {
    toks: Vec<Token>,
    i: usize,
    file: &'a str,
    src: &'a str,
}

type PResult<T> = Result<T, CompileError>;

pub fn parse(file: &str, src: &str, toks: Vec<Token>, program: &mut Program) -> PResult<()> {
    let mut p = Parser { toks, i: 0, file, src };
    program.files.push(SourceFile { name: file.to_string(), source: src.to_string() });
    loop {
        match p.peek().clone() {
            Tok::Eof => return Ok(()),
            Tok::Kw(Kw::Let) => {
                let l = p.let_decl()?;
                program.lets.push(l);
            }
            Tok::Kw(Kw::Model) => {
                let m = p.model()?;
                program.models.push(m);
            }
            Tok::Kw(Kw::Seed) => {
                let s = p.seed()?;
                program.seeds.push(s);
            }
            Tok::Kw(Kw::Component) => {
                let c = p.component()?;
                program.components.push(c);
            }
            Tok::Kw(Kw::Page) => {
                let pg = p.page()?;
                program.pages.push(pg);
            }
            other => {
                return Err(p
                    .err_here(format!("expected 'let', 'model', 'seed', 'component' or 'page', found {}", other.describe()))
                    .hint("a .complex file is a list of declarations"));
            }
        }
    }
}

impl<'a> Parser<'a> {
    // ---- token helpers ------------------------------------------------------

    fn peek(&self) -> &Tok {
        &self.toks[self.i].tok
    }

    fn peek_at(&self, off: usize) -> &Tok {
        let j = (self.i + off).min(self.toks.len() - 1);
        &self.toks[j].tok
    }

    fn pos(&self) -> Pos {
        self.toks[self.i].pos
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        if self.i < self.toks.len() - 1 {
            self.i += 1;
        }
        t
    }

    fn err_here(&self, msg: impl Into<String>) -> CompileError {
        CompileError::new(self.file, self.src, self.pos(), msg)
    }

    fn is_punct(&self, p: &str) -> bool {
        matches!(self.peek(), Tok::Punct(q) if *q == p)
    }

    fn eat_punct(&mut self, p: &str) -> bool {
        if self.is_punct(p) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn expect_punct(&mut self, p: &str) -> PResult<Pos> {
        if self.is_punct(p) {
            Ok(self.advance().pos)
        } else {
            let err = self.err_here(format!("expected '{p}', found {}", self.peek().describe()));
            Err(match p {
                "}" => err.hint("every '{' needs a matching '}'"),
                ")" => err.hint("every '(' needs a matching ')'"),
                _ => err,
            })
        }
    }

    fn expect_kw(&mut self, k: Kw) -> PResult<Pos> {
        if self.peek() == &Tok::Kw(k) {
            Ok(self.advance().pos)
        } else {
            Err(self.err_here(format!("expected '{}', found {}", kw_text(k), self.peek().describe())))
        }
    }

    fn expect_name(&mut self, what: &str) -> PResult<(String, Pos)> {
        match self.peek().clone() {
            Tok::Ident(n) => {
                let pos = self.advance().pos;
                Ok((n, pos))
            }
            Tok::Kw(k) => Err(self
                .err_here(format!("'{}' is a keyword and cannot be used as {what}", kw_text(k)))
                .hint("choose another name")),
            other => Err(self.err_here(format!("expected {what}, found {}", other.describe()))),
        }
    }

    // ---- declarations -------------------------------------------------------

    fn let_decl(&mut self) -> PResult<Let> {
        let pos = self.expect_kw(Kw::Let)?;
        let (name, _) = self.expect_name("a name after 'let'")?;
        self.expect_punct("=")?;
        let value = self.expr()?;
        Ok(Let { pos, file: self.file.to_string(), name, value })
    }

    fn model(&mut self) -> PResult<Model> {
        let pos = self.expect_kw(Kw::Model)?;
        let (name, npos) = self.expect_name("a model name")?;
        if !name.chars().next().is_some_and(|c| c.is_uppercase()) {
            return Err(CompileError::new(self.file, self.src, npos, format!("model names start with a capital: '{name}'"))
                .hint("models are types, like Citizen or Artwork"));
        }
        self.expect_punct("{")?;
        let mut fields = Vec::new();
        while !self.is_punct("}") {
            if self.peek() == &Tok::Eof {
                return Err(self.err_here("model is never closed").hint("add a closing '}'"));
            }
            let (fname, fpos) = self.expect_name("a field name")?;
            self.expect_punct(":").map_err(|e| e.hint("fields need a type, like `name: Text`"))?;
            let ty = self.type_expr()?;
            let label = if self.eat_punct("@") {
                let (l, lpos) = self.expect_name("a label after '@'")?;
                Some((l, lpos))
            } else {
                None
            };
            fields.push(ModelField { pos: fpos, name: fname, ty, label });
            self.eat_punct(",");
        }
        self.expect_punct("}")?;
        Ok(Model { pos, file: self.file.to_string(), name, fields })
    }

    fn seed(&mut self) -> PResult<Seed> {
        let pos = self.expect_kw(Kw::Seed)?;
        let (model, _) = self.expect_name("a model name after 'seed'")?;
        let value = self.expr()?;
        Ok(Seed { pos, file: self.file.to_string(), model, value })
    }

    fn params(&mut self) -> PResult<Vec<Param>> {
        self.expect_punct("(")?;
        let mut params = Vec::new();
        while !self.is_punct(")") {
            let (pname, ppos) = self.expect_name("a parameter name")?;
            self.expect_punct(":").map_err(|e| e.hint("parameters need a type, like `title: Text`"))?;
            let ty = self.type_expr()?;
            params.push(Param { pos: ppos, name: pname, ty });
            if !self.eat_punct(",") {
                break;
            }
        }
        self.expect_punct(")")?;
        Ok(params)
    }

    fn component(&mut self) -> PResult<Component> {
        let pos = self.expect_kw(Kw::Component)?;
        let (name, _) = self.expect_name("a component name")?;
        let params = self.params()?;
        let body = self.nodes()?;
        Ok(Component { pos, file: self.file.to_string(), name, params, body })
    }

    fn type_expr(&mut self) -> PResult<TypeExpr> {
        if self.eat_punct("[") {
            let inner = self.type_expr()?;
            self.expect_punct("]")?;
            return Ok(TypeExpr::List(Box::new(inner)));
        }
        let (name, _) = self.expect_name("a type")?;
        if self.eat_punct("<") {
            let inner = self.type_expr()?;
            self.expect_punct(">")?;
            return Ok(TypeExpr::Generic(name, Box::new(inner)));
        }
        Ok(TypeExpr::Named(name))
    }

    fn page(&mut self) -> PResult<Page> {
        let pos = self.expect_kw(Kw::Page)?;
        let pattern = match self.peek().clone() {
            Tok::Str(s) => {
                self.advance();
                s
            }
            other => {
                return Err(self
                    .err_here(format!("expected the page address as text, found {}", other.describe()))
                    .hint("write it like: page \"/about\" { ... }"));
            }
        };
        if !pattern.starts_with('/') {
            return Err(CompileError::new(self.file, self.src, pos, format!("page address \"{pattern}\" must start with '/'")));
        }
        let params = if self.is_punct("(") { self.params()? } else { Vec::new() };
        self.expect_punct("{")?;
        let mut stmts = Vec::new();
        loop {
            match self.peek() {
                Tok::Kw(Kw::Let) => stmts.push(Stmt::Let(self.let_decl()?)),
                Tok::Kw(Kw::Log) => {
                    let lpos = self.advance().pos;
                    let value = self.expr()?;
                    stmts.push(Stmt::Log { pos: lpos, value });
                }
                _ => break,
            }
        }
        self.expect_kw(Kw::View).map_err(|e| e.hint("a page ends with a `view { ... }` block"))?;
        let view = self.nodes()?;
        self.expect_punct("}")?;
        Ok(Page { pos, file: self.file.to_string(), pattern, params, stmts, view })
    }

    // ---- view nodes ---------------------------------------------------------

    fn nodes(&mut self) -> PResult<Vec<Node>> {
        self.expect_punct("{")?;
        let mut out = Vec::new();
        while !self.is_punct("}") {
            if self.peek() == &Tok::Eof {
                return Err(self.err_here("block is never closed").hint("add a closing '}'"));
            }
            out.push(self.node()?);
            self.eat_punct(",");
        }
        self.expect_punct("}")?;
        Ok(out)
    }

    fn node(&mut self) -> PResult<Node> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Kw(Kw::For) => {
                self.advance();
                let (var, _) = self.expect_name("a loop variable")?;
                self.expect_kw(Kw::In)?;
                let list = self.expr()?;
                let body = self.nodes()?;
                Ok(Node::For { pos, var, list, body })
            }
            Tok::Kw(Kw::If) => self.if_node(),
            Tok::Ident(name) => {
                let next = self.peek_at(1).clone();
                let is_builtin = BUILTINS.contains(&name.as_str());
                if name == "children" && !matches!(next, Tok::Punct(".") | Tok::Punct("+")) {
                    self.advance();
                    return Ok(Node::Children { pos });
                }
                let starts_element = matches!(next, Tok::Punct("(") | Tok::Punct("{")) && !is_builtin;
                let bare_void = VOID_TAGS.contains(&name.as_str())
                    && !matches!(next, Tok::Punct(".") | Tok::Punct("+") | Tok::Punct("==") | Tok::Punct("!="));
                if starts_element || bare_void {
                    self.element()
                } else {
                    Ok(Node::Text(self.expr()?))
                }
            }
            _ => Ok(Node::Text(self.expr()?)),
        }
    }

    fn if_node(&mut self) -> PResult<Node> {
        let pos = self.expect_kw(Kw::If)?;
        let cond = self.expr()?;
        let then = self.nodes()?;
        let mut otherwise = Vec::new();
        if self.peek() == &Tok::Kw(Kw::Else) {
            self.advance();
            if self.peek() == &Tok::Kw(Kw::If) {
                otherwise.push(self.if_node()?);
            } else {
                otherwise = self.nodes()?;
            }
        }
        Ok(Node::If { pos, cond, then, otherwise })
    }

    fn element(&mut self) -> PResult<Node> {
        let (name, pos) = self.expect_name("an element name")?;
        let mut args = Vec::new();
        if self.eat_punct("(") {
            while !self.is_punct(")") {
                args.push(self.arg()?);
                if !self.eat_punct(",") {
                    break;
                }
            }
            self.expect_punct(")")?;
        }
        let has_body = self.is_punct("{");
        let children = if has_body { self.nodes()? } else { Vec::new() };
        Ok(Node::Element { pos, name, args, children, has_body })
    }

    /// `name: value` or `"aria-label": value`.
    fn arg(&mut self) -> PResult<Arg> {
        let pos = self.pos();
        let name = match self.peek().clone() {
            Tok::Ident(n) => {
                self.advance();
                n
            }
            Tok::Str(s) => {
                self.advance();
                s
            }
            other => {
                return Err(self
                    .err_here(format!("expected a named argument, found {}", other.describe()))
                    .hint("arguments are named: `href: \"/about\"`"));
            }
        };
        self.expect_punct(":").map_err(|e| e.hint("arguments are named: `href: \"/about\"`"))?;
        let value = self.expr()?;
        Ok(Arg { pos, name, value })
    }

    // ---- expressions --------------------------------------------------------

    pub fn expr(&mut self) -> PResult<Expr> {
        let mut left = self.sum()?;
        loop {
            let op = match self.peek() {
                Tok::Punct("==") => BinOp::Eq,
                Tok::Punct("!=") => BinOp::NotEq,
                _ => return Ok(left),
            };
            let pos = self.advance().pos;
            let right = self.sum()?;
            left = Expr::Binary { pos, op, left: Box::new(left), right: Box::new(right) };
        }
    }

    fn sum(&mut self) -> PResult<Expr> {
        let mut left = self.postfix()?;
        while self.is_punct("+") {
            let pos = self.advance().pos;
            let right = self.postfix()?;
            left = Expr::Binary { pos, op: BinOp::Add, left: Box::new(left), right: Box::new(right) };
        }
        Ok(left)
    }

    fn postfix(&mut self) -> PResult<Expr> {
        let mut e = self.primary()?;
        loop {
            if self.is_punct(".") {
                let pos = self.advance().pos;
                let (name, _) = self.expect_name("a field name after '.'")?;
                e = Expr::Field { pos, target: Box::new(e), name };
            } else if self.is_punct("[") {
                let pos = self.advance().pos;
                let index = self.expr()?;
                self.expect_punct("]")?;
                e = Expr::Index { pos, target: Box::new(e), index: Box::new(index) };
            } else {
                return Ok(e);
            }
        }
    }

    fn primary(&mut self) -> PResult<Expr> {
        let pos = self.pos();
        match self.peek().clone() {
            Tok::Str(value) => {
                self.advance();
                Ok(Expr::Str { pos, value })
            }
            Tok::Int(value) => {
                self.advance();
                Ok(Expr::Int { pos, value })
            }
            Tok::Kw(Kw::True) => {
                self.advance();
                Ok(Expr::Bool { pos, value: true })
            }
            Tok::Kw(Kw::False) => {
                self.advance();
                Ok(Expr::Bool { pos, value: false })
            }
            Tok::Punct("[") => {
                self.advance();
                let mut items = Vec::new();
                while !self.is_punct("]") {
                    items.push(self.expr()?);
                    if !self.eat_punct(",") {
                        break;
                    }
                }
                self.expect_punct("]")?;
                Ok(Expr::List { pos, items })
            }
            Tok::Punct("{") => {
                self.advance();
                let mut fields = Vec::new();
                while !self.is_punct("}") {
                    fields.push(self.arg()?);
                    self.eat_punct(",");
                }
                self.expect_punct("}")?;
                Ok(Expr::Record { pos, fields })
            }
            Tok::Punct("(") => {
                self.advance();
                let e = self.expr()?;
                self.expect_punct(")")?;
                Ok(e)
            }
            Tok::Ident(name) => {
                self.advance();
                if self.eat_punct("(") {
                    let mut args = Vec::new();
                    while !self.is_punct(")") {
                        args.push(self.expr()?);
                        if !self.eat_punct(",") {
                            break;
                        }
                    }
                    self.expect_punct(")")?;
                    Ok(Expr::Call { pos, name, args })
                } else {
                    Ok(Expr::Name { pos, name })
                }
            }
            other => Err(self.err_here(format!("expected a value, found {}", other.describe()))),
        }
    }
}

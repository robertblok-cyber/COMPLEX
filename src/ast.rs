// RobertBlok.com — Fullstack development
//
// The abstract syntax tree: the shape of a Complex program after parsing.
// Every node is an enum variant, so the compiler forces every pass
// (checker, interpreter) to handle every kind of node.

use crate::error::Pos;

#[derive(Debug, Default)]
pub struct Program {
    pub files: Vec<SourceFile>,
    pub lets: Vec<Let>,
    pub components: Vec<Component>,
    pub pages: Vec<Page>,
}

#[derive(Debug, Clone)]
pub struct SourceFile {
    pub name: String,
    pub source: String,
}

#[derive(Debug, Clone)]
pub struct Let {
    pub pos: Pos,
    pub file: String,
    pub name: String,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub pos: Pos,
    pub name: String,
    pub ty: TypeExpr,
}

/// A type as written in source: `Text`, `[Text]`, `Secret<Text>`.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeExpr {
    Named(String),
    List(Box<TypeExpr>),
    Generic(String, Box<TypeExpr>),
}

#[derive(Debug, Clone)]
pub struct Component {
    pub pos: Pos,
    pub file: String,
    pub name: String,
    pub params: Vec<Param>,
    pub body: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Page {
    pub pos: Pos,
    pub file: String,
    pub pattern: String,
    pub lets: Vec<Let>,
    pub view: Vec<Node>,
}

#[derive(Debug, Clone)]
pub struct Arg {
    pub pos: Pos,
    pub name: String,
    pub value: Expr,
}

#[derive(Debug, Clone)]
pub enum Node {
    /// An HTML tag or a component call; the checker decides which.
    Element { pos: Pos, name: String, args: Vec<Arg>, children: Vec<Node>, has_body: bool },
    /// The value of an expression, rendered as HTML-escaped text.
    Text(Expr),
    For { pos: Pos, var: String, list: Expr, body: Vec<Node> },
    If { pos: Pos, cond: Expr, then: Vec<Node>, otherwise: Vec<Node> },
    /// Inside a component: the nodes the caller passed between its braces.
    Children { pos: Pos },
}

#[derive(Debug, Clone)]
pub enum Expr {
    Str { pos: Pos, value: String },
    Int { pos: Pos, value: i64 },
    Bool { pos: Pos, value: bool },
    List { pos: Pos, items: Vec<Expr> },
    Record { pos: Pos, fields: Vec<Arg> },
    Name { pos: Pos, name: String },
    Field { pos: Pos, target: Box<Expr>, name: String },
    Call { pos: Pos, name: String, args: Vec<Expr> },
    Binary { pos: Pos, op: BinOp, left: Box<Expr>, right: Box<Expr> },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Eq,
    NotEq,
}

impl Expr {
    pub fn pos(&self) -> Pos {
        match self {
            Expr::Str { pos, .. }
            | Expr::Int { pos, .. }
            | Expr::Bool { pos, .. }
            | Expr::List { pos, .. }
            | Expr::Record { pos, .. }
            | Expr::Name { pos, .. }
            | Expr::Field { pos, .. }
            | Expr::Call { pos, .. }
            | Expr::Binary { pos, .. } => *pos,
        }
    }
}

/// Built-in functions of the core language.
pub const BUILTINS: [&str; 6] = ["now", "year", "len", "upper", "secret", "masked"];

/// HTML elements that never have children.
pub const VOID_TAGS: [&str; 8] = ["br", "hr", "img", "input", "meta", "link", "wbr", "source"];

/// HTML elements Complex knows; anything else must be a component.
pub const HTML_TAGS: [&str; 62] = [
    "html", "head", "body", "title", "meta", "link", "style", "script", "main", "header", "footer",
    "nav", "section", "article", "aside", "div", "span", "p", "a", "h1", "h2", "h3", "h4", "h5",
    "h6", "ul", "ol", "li", "dl", "dt", "dd", "strong", "em", "small", "code", "pre", "kbd",
    "blockquote", "figure", "figcaption", "img", "picture", "source", "video", "audio", "br", "hr",
    "wbr", "table", "thead", "tbody", "tr", "th", "td", "form", "label", "input", "button",
    "textarea", "select", "option", "time",
];

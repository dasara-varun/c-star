use crate::diagnostics::Span;

#[derive(Debug, Clone, PartialEq)]
pub enum TypeKind {
    Path(Vec<String>, Vec<Type>), // e.g. std::Result<T, E> (path components, generic args)
    Ref(Box<Type>),              // &T
    Slice(Box<Type>),            // []T
    RawPtr(Box<Type>),           // raw *T
}

#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    pub kind: TypeKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Int(i64, Option<String>),
    Float(f64, Option<String>),
    Str(String),
    Bool(bool),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprKind {
    Lit(Lit),
    Ident(String),
    Path(Vec<String>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Unary(UnOp, Box<Expr>),
    FieldAccess(Box<Expr>, String),
    MethodCall(Box<Expr>, String, Vec<Expr>),
    Call(Box<Expr>, Vec<Expr>),
    StructInit(Vec<String>, Vec<(String, Expr)>), // Path, fields (name, value)
    Block(Block),
    If(Box<Expr>, Block, Option<BlockOrIf>),
    Match(Box<Expr>, Vec<MatchArm>),
    Try(Box<Expr>), // expr?
    Index(Box<Expr>, Box<Expr>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BinOp {
    Add, Sub, Mul, Div,
    Eq, Ne, Lt, Gt, Le, Ge,
    And, Or,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UnOp {
    AddrOf, // &
    Deref,  // *
    Not,    // !
}

#[derive(Debug, Clone, PartialEq)]
pub enum BlockOrIf {
    Block(Block),
    If(Box<Expr>, Block, Option<Box<BlockOrIf>>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Expr,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    Ident(String),
    Struct(Vec<String>, Vec<(String, Pattern)>), // Path, fields
    EnumVariant(Vec<String>, Vec<Pattern>),      // Path::Variant(args)
    Lit(Lit),
    Underscore,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum StmtKind {
    Let {
        is_mut: bool,
        name: String,
        ty: Option<Type>,
        value: Expr,
    },
    Assign(Expr, Expr), // dest = src
    Expr(Expr),
    Return(Option<Expr>),
    While(Expr, Block),
    For(String, Expr, Block), // for name in iter block
    Raw(Block), // raw { ... }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Param {
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FnDecl {
    pub is_pub: bool,
    pub name: String,
    pub generic_params: Vec<String>,
    pub params: Vec<Param>,
    pub ret_ty: Option<Type>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructField {
    pub is_pub: bool,
    pub name: String,
    pub ty: Type,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StructDecl {
    pub is_pub: bool,
    pub name: String,
    pub generic_params: Vec<String>,
    pub fields: Vec<StructField>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumVariant {
    pub name: String,
    pub types: Vec<Type>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EnumDecl {
    pub is_pub: bool,
    pub name: String,
    pub generic_params: Vec<String>,
    pub variants: Vec<EnumVariant>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImplBlock {
    pub target_ty: Type,
    pub methods: Vec<FnDecl>,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    Fn(FnDecl),
    Struct(StructDecl),
    Enum(EnumDecl),
    Impl(ImplBlock),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub name: Option<String>, // Declared module name
    pub imports: Vec<Import>,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: Vec<String>,
    pub alias: Option<String>,
    pub span: Span,
}

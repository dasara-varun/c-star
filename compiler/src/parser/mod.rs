use crate::ast::*;
use crate::diagnostics::{Diagnostic, Span};
use crate::lexer::{Lexer, Token, TokenKind};

pub struct Parser<'a> {
    lexer: Lexer<'a>,
    current_token: Token,
    peek_token: Token,
    pub diagnostics: Vec<Diagnostic>,
    filename: String,
    line: usize,
    col: usize,
}

impl<'a> Parser<'a> {
    pub fn new(mut lexer: Lexer<'a>, filename: &str) -> Result<Self, String> {
        let current_token = lexer.next_token()?;
        let peek_token = lexer.next_token()?;
        let line = lexer.line();
        let col = lexer.col();
        Ok(Self {
            lexer,
            current_token,
            peek_token,
            diagnostics: Vec::new(),
            filename: filename.to_string(),
            line,
            col,
        })
    }

    fn advance(&mut self) -> Result<(), String> {
        self.current_token = std::mem::replace(&mut self.peek_token, self.lexer.next_token()?);
        self.line = self.lexer.line();
        self.col = self.lexer.col();
        Ok(())
    }

    fn check(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.current_token.kind) == std::mem::discriminant(kind)
    }

    fn check_peek(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek_token.kind) == std::mem::discriminant(kind)
    }

    fn expect(&mut self, kind: TokenKind) -> Result<Token, String> {
        if self.check(&kind) {
            let tok = self.current_token.clone();
            self.advance()?;
            Ok(tok)
        } else {
            let span = self.current_token.span;
            let msg = format!(
                "Expected token {:?}, found {:?}",
                kind, self.current_token.kind
            );
            self.error("E0001", msg, span);
            Err("Parsing error".to_string())
        }
    }

    fn error(&mut self, code: &str, message: String, span: Span) {
        self.diagnostics.push(Diagnostic {
            code: code.to_string(),
            severity: "error".to_string(),
            message,
            file: self.filename.clone(),
            span,
            notes: Vec::new(),
            suggested_fix: None,
        });
    }

    fn recover_to_stmt_boundary(&mut self) -> Result<(), String> {
        loop {
            match self.current_token.kind {
                TokenKind::Semicolon | TokenKind::EOF => {
                    self.advance()?;
                    break;
                }
                TokenKind::Let
                | TokenKind::Return
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Pub
                | TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Impl => {
                    break;
                }
                _ => {
                    self.advance()?;
                }
            }
        }
        Ok(())
    }

    pub fn parse_module(&mut self) -> Result<Module, String> {
        let mut name = None;
        if self.check(&TokenKind::Module) {
            self.advance()?;
            let ident_tok = self.expect(TokenKind::Ident(String::new()))?;
            if let TokenKind::Ident(n) = ident_tok.kind {
                name = Some(n);
            }
            self.expect(TokenKind::Semicolon)?;
        }

        let mut imports = Vec::new();
        while self.check(&TokenKind::Use) {
            match self.parse_import() {
                Ok(imp) => imports.push(imp),
                Err(_) => {
                    self.recover_to_stmt_boundary()?;
                }
            }
        }

        let mut items = Vec::new();
        while !self.check(&TokenKind::EOF) {
            if self.check(&TokenKind::Semicolon) {
                self.advance()?;
                continue;
            }
            match self.parse_item() {
                Ok(item) => items.push(item),
                Err(_) => {
                    self.recover_to_stmt_boundary()?;
                }
            }
        }

        Ok(Module {
            name,
            imports,
            items,
        })
    }

    fn parse_import(&mut self) -> Result<Import, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::Use)?;

        let mut path = Vec::new();
        let first_ident = self.expect(TokenKind::Ident(String::new()))?;
        if let TokenKind::Ident(id) = first_ident.kind {
            path.push(id);
        }

        while self.check(&TokenKind::Dot) || self.check(&TokenKind::ColonColon) {
            self.advance()?;
            let id = self.expect(TokenKind::Ident(String::new()))?;
            if let TokenKind::Ident(n) = id.kind {
                path.push(n);
            }
        }

        let mut alias = None;
        if self.check(&TokenKind::As) {
            self.advance()?;
            let alias_ident = self.expect(TokenKind::Ident(String::new()))?;
            if let TokenKind::Ident(n) = alias_ident.kind {
                alias = Some(n);
            }
        }

        self.expect(TokenKind::Semicolon)?;

        Ok(Import {
            path,
            alias,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_item(&mut self) -> Result<Item, String> {
        let is_pub = if self.check(&TokenKind::Pub) {
            self.advance()?;
            true
        } else {
            false
        };

        if self.check(&TokenKind::Fn) {
            Ok(Item::Fn(self.parse_fn_decl(is_pub)?))
        } else if self.check(&TokenKind::Struct) {
            Ok(Item::Struct(self.parse_struct_decl(is_pub)?))
        } else if self.check(&TokenKind::Enum) {
            Ok(Item::Enum(self.parse_enum_decl(is_pub)?))
        } else if self.check(&TokenKind::Impl) {
            if is_pub {
                self.error("E0002", "impl blocks cannot be declared public".to_string(), self.current_token.span);
            }
            Ok(Item::Impl(self.parse_impl_block()?))
        } else {
            let span = self.current_token.span;
            let msg = format!("Expected fn, struct, enum, or impl, found {:?}", self.current_token.kind);
            self.error("E0003", msg, span);
            self.advance()?;
            Err("Invalid item".to_string())
        }
    }

    fn parse_fn_decl(&mut self, is_pub: bool) -> Result<FnDecl, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::Fn)?;
        
        let ident_tok = self.expect(TokenKind::Ident(String::new()))?;
        let name = match ident_tok.kind {
            TokenKind::Ident(n) => n,
            _ => unreachable!(),
        };

        let mut generic_params = Vec::new();
        if self.check(&TokenKind::Lt) {
            self.advance()?;
            loop {
                let id_tok = self.expect(TokenKind::Ident(String::new()))?;
                if let TokenKind::Ident(n) = id_tok.kind {
                    generic_params.push(n);
                }
                if self.check(&TokenKind::Comma) {
                    self.advance()?;
                } else {
                    break;
                }
            }
            self.expect(TokenKind::Gt)?;
        }

        self.expect(TokenKind::LParen)?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                let p_span = self.current_token.span;
                let is_self = match &self.current_token.kind {
                    TokenKind::Ident(n) => n == "self",
                    _ => false,
                };
                let p_name = if is_self {
                    self.advance()?;
                    "self".to_string()
                } else {
                    let id_tok = self.expect(TokenKind::Ident(String::new()))?;
                    match id_tok.kind {
                        TokenKind::Ident(n) => n,
                        _ => unreachable!(),
                    }
                };

                // For self, type is optional
                let p_ty = if is_self && !self.check(&TokenKind::Colon) {
                    // Implicit self type (handled during type checking)
                    Type {
                        kind: TypeKind::Path(vec!["Self".to_string()], Vec::new()),
                        span: p_span,
                    }
                } else {
                    self.expect(TokenKind::Colon)?;
                    self.parse_type()?
                };

                params.push(Param {
                    name: p_name,
                    ty: p_ty,
                    span: p_span,
                });

                if self.check(&TokenKind::Comma) {
                    self.advance()?;
                } else {
                    break;
                }
            }
        }
        self.expect(TokenKind::RParen)?;

        let mut ret_ty = None;
        if self.check(&TokenKind::Arrow) {
            self.advance()?;
            ret_ty = Some(self.parse_type()?);
        }

        let body = self.parse_block()?;

        Ok(FnDecl {
            is_pub,
            name,
            generic_params,
            params,
            ret_ty,
            body,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_struct_decl(&mut self, is_pub: bool) -> Result<StructDecl, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::Struct)?;

        let ident_tok = self.expect(TokenKind::Ident(String::new()))?;
        let name = match ident_tok.kind {
            TokenKind::Ident(n) => n,
            _ => unreachable!(),
        };

        let mut generic_params = Vec::new();
        if self.check(&TokenKind::Lt) {
            self.advance()?;
            loop {
                let id_tok = self.expect(TokenKind::Ident(String::new()))?;
                if let TokenKind::Ident(n) = id_tok.kind {
                    generic_params.push(n);
                }
                if self.check(&TokenKind::Comma) {
                    self.advance()?;
                } else {
                    break;
                }
            }
            self.expect(TokenKind::Gt)?;
        }

        self.expect(TokenKind::LBrace)?;
        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
            let f_span = self.current_token.span;
            let f_pub = if self.check(&TokenKind::Pub) {
                self.advance()?;
                true
            } else {
                false
            };

            let f_ident = self.expect(TokenKind::Ident(String::new()))?;
            let f_name = match f_ident.kind {
                TokenKind::Ident(n) => n,
                _ => unreachable!(),
            };

            self.expect(TokenKind::Colon)?;
            let f_ty = self.parse_type()?;

            fields.push(StructField {
                is_pub: f_pub,
                name: f_name,
                ty: f_ty,
                span: f_span,
            });

            if self.check(&TokenKind::Comma) {
                self.advance()?;
            } else if self.check(&TokenKind::Semicolon) {
                self.advance()?;
            }
        }
        self.expect(TokenKind::RBrace)?;

        Ok(StructDecl {
            is_pub,
            name,
            generic_params,
            fields,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_enum_decl(&mut self, is_pub: bool) -> Result<EnumDecl, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::Enum)?;

        let ident_tok = self.expect(TokenKind::Ident(String::new()))?;
        let name = match ident_tok.kind {
            TokenKind::Ident(n) => n,
            _ => unreachable!(),
        };

        let mut generic_params = Vec::new();
        if self.check(&TokenKind::Lt) {
            self.advance()?;
            loop {
                let id_tok = self.expect(TokenKind::Ident(String::new()))?;
                if let TokenKind::Ident(n) = id_tok.kind {
                    generic_params.push(n);
                }
                if self.check(&TokenKind::Comma) {
                    self.advance()?;
                } else {
                    break;
                }
            }
            self.expect(TokenKind::Gt)?;
        }

        self.expect(TokenKind::LBrace)?;
        let mut variants = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
            let v_span = self.current_token.span;
            let v_ident = self.expect(TokenKind::Ident(String::new()))?;
            let v_name = match v_ident.kind {
                TokenKind::Ident(n) => n,
                _ => unreachable!(),
            };

            let mut types = Vec::new();
            if self.check(&TokenKind::LParen) {
                self.advance()?;
                loop {
                    types.push(self.parse_type()?);
                    if self.check(&TokenKind::Comma) {
                        self.advance()?;
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::RParen)?;
            }

            variants.push(EnumVariant {
                name: v_name,
                types,
                span: v_span,
            });

            if self.check(&TokenKind::Comma) {
                self.advance()?;
            } else if self.check(&TokenKind::Semicolon) {
                self.advance()?;
            }
        }
        self.expect(TokenKind::RBrace)?;

        Ok(EnumDecl {
            is_pub,
            name,
            generic_params,
            variants,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_impl_block(&mut self) -> Result<ImplBlock, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::Impl)?;

        let target_ty = self.parse_type()?;
        self.expect(TokenKind::LBrace)?;

        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
            if self.check(&TokenKind::Semicolon) {
                self.advance()?;
                continue;
            }
            let is_m_pub = if self.check(&TokenKind::Pub) {
                self.advance()?;
                true
            } else {
                false
            };

            methods.push(self.parse_fn_decl(is_m_pub)?);
        }
        self.expect(TokenKind::RBrace)?;

        Ok(ImplBlock {
            target_ty,
            methods,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_type(&mut self) -> Result<Type, String> {
        let start_span = self.current_token.span;
        if self.check(&TokenKind::Amp) {
            self.advance()?;
            let inner = self.parse_type()?;
            Ok(Type {
                kind: TypeKind::Ref(Box::new(inner)),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::LBracket) {
            self.advance()?;
            self.expect(TokenKind::RBracket)?;
            let inner = self.parse_type()?;
            Ok(Type {
                kind: TypeKind::Slice(Box::new(inner)),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::Raw) {
            self.advance()?;
            self.expect(TokenKind::Star)?;
            let inner = self.parse_type()?;
            Ok(Type {
                kind: TypeKind::RawPtr(Box::new(inner)),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else {
            // Path type with generic args
            let mut path_parts = Vec::new();
            let first_ident = self.expect(TokenKind::Ident(String::new()))?;
            if let TokenKind::Ident(n) = first_ident.kind {
                path_parts.push(n);
            }

            while self.check(&TokenKind::ColonColon) {
                self.advance()?;
                let id = self.expect(TokenKind::Ident(String::new()))?;
                if let TokenKind::Ident(n) = id.kind {
                    path_parts.push(n);
                }
            }

            let mut gen_args = Vec::new();
            if self.check(&TokenKind::Lt) {
                self.advance()?;
                loop {
                    gen_args.push(self.parse_type()?);
                    if self.check(&TokenKind::Comma) {
                        self.advance()?;
                    } else {
                        break;
                    }
                }
                self.expect(TokenKind::Gt)?;
            }

            Ok(Type {
                kind: TypeKind::Path(path_parts, gen_args),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        }
    }

    fn parse_block(&mut self) -> Result<Block, String> {
        let start_span = self.current_token.span;
        self.expect(TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
            match self.parse_stmt() {
                Ok(stmt) => stmts.push(stmt),
                Err(_) => {
                    self.recover_to_stmt_boundary()?;
                }
            }
        }
        self.expect(TokenKind::RBrace)?;

        Ok(Block {
            stmts,
            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
        })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, String> {
        let start_span = self.current_token.span;
        if self.check(&TokenKind::Semicolon) {
            self.advance()?;
            return Ok(Stmt {
                kind: StmtKind::Expr(Expr {
                    kind: ExprKind::Block(Block {
                        stmts: Vec::new(),
                        span: start_span,
                    }),
                    span: start_span,
                }),
                span: start_span,
            });
        }
        if self.check(&TokenKind::Let) {
            self.advance()?;
            let is_mut = if self.check(&TokenKind::Mut) {
                self.advance()?;
                true
            } else {
                false
            };

            let name_tok = self.expect(TokenKind::Ident(String::new()))?;
            let name = match name_tok.kind {
                TokenKind::Ident(n) => n,
                _ => unreachable!(),
            };

            let mut ty = None;
            if self.check(&TokenKind::Colon) {
                self.advance()?;
                ty = Some(self.parse_type()?);
            }

            self.expect(TokenKind::Eq)?;
            let value = self.parse_expr()?;
            self.expect(TokenKind::Semicolon)?;

            Ok(Stmt {
                kind: StmtKind::Let { is_mut, name, ty, value },
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::Return) {
            self.advance()?;
            let mut value = None;
            if !self.check(&TokenKind::Semicolon) {
                value = Some(self.parse_expr()?);
            }
            self.expect(TokenKind::Semicolon)?;

            Ok(Stmt {
                kind: StmtKind::Return(value),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::While) {
            self.advance()?;
            let cond = self.parse_expr()?;
            let body = self.parse_block()?;
            Ok(Stmt {
                kind: StmtKind::While(cond, body),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::For) {
            self.advance()?;
            let var_tok = self.expect(TokenKind::Ident(String::new()))?;
            let var_name = match var_tok.kind {
                TokenKind::Ident(n) => n,
                _ => unreachable!(),
            };
            self.expect(TokenKind::In)?;
            let iter = self.parse_expr()?;
            let body = self.parse_block()?;
            Ok(Stmt {
                kind: StmtKind::For(var_name, iter, body),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else if self.check(&TokenKind::Raw) && self.check_peek(&TokenKind::LBrace) {
            // raw block
            self.advance()?; // raw
            let body = self.parse_block()?;
            Ok(Stmt {
                kind: StmtKind::Raw(body),
                span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
            })
        } else {
            // Expression statement or assignment
            let expr = self.parse_expr()?;
            if self.check(&TokenKind::Eq) {
                self.advance()?;
                let rhs = self.parse_expr()?;
                self.expect(TokenKind::Semicolon)?;
                Ok(Stmt {
                    kind: StmtKind::Assign(expr, rhs),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                })
            } else {
                self.expect(TokenKind::Semicolon)?;
                Ok(Stmt {
                    kind: StmtKind::Expr(expr),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                })
            }
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, String> {
        self.parse_expr_bp(0)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> Result<Expr, String> {
        let start_span = self.current_token.span;
        
        // Null Denotation (Prefix/Primary)
        let mut lhs = match &self.current_token.kind {
            TokenKind::Ident(_) | TokenKind::ColonColon => {
                // Ident, Path or Struct instantiation
                let mut path = Vec::new();
                if self.check(&TokenKind::Ident(String::new())) {
                    if let TokenKind::Ident(n) = &self.current_token.kind {
                        path.push(n.clone());
                    }
                    self.advance()?;
                }

                while self.check(&TokenKind::ColonColon) {
                    self.advance()?;
                    let id = self.expect(TokenKind::Ident(String::new()))?;
                    if let TokenKind::Ident(n) = id.kind {
                        path.push(n);
                    }
                }

                let span = Span::new(start_span.start_line, start_span.start_col, self.line, self.col);
                if self.check(&TokenKind::LBrace) {
                    // Struct instantiation! E.g. Point { x: x, y: y }
                    self.advance()?;
                    let mut fields = Vec::new();
                    while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
                        let f_name_tok = self.expect(TokenKind::Ident(String::new()))?;
                        let f_name = match f_name_tok.kind {
                            TokenKind::Ident(n) => n,
                            _ => unreachable!(),
                        };
                        self.expect(TokenKind::Colon)?;
                        let f_val = self.parse_expr()?;
                        fields.push((f_name, f_val));

                        if self.check(&TokenKind::Comma) {
                            self.advance()?;
                        } else {
                            break;
                        }
                    }
                    self.expect(TokenKind::RBrace)?;
                    let end_span = Span::new(start_span.start_line, start_span.start_col, self.line, self.col);
                    Expr {
                        kind: ExprKind::StructInit(path, fields),
                        span: end_span,
                    }
                } else {
                    if path.len() == 1 {
                        Expr {
                            kind: ExprKind::Ident(path[0].clone()),
                            span,
                        }
                    } else {
                        Expr {
                            kind: ExprKind::Path(path),
                            span,
                        }
                    }
                }
            }
            TokenKind::Int(val, suffix) => {
                let v = *val;
                let s = suffix.clone();
                self.advance()?;
                Expr {
                    kind: ExprKind::Lit(Lit::Int(v, s)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Float(val, suffix) => {
                let v = *val;
                let s = suffix.clone();
                self.advance()?;
                Expr {
                    kind: ExprKind::Lit(Lit::Float(v, s)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Str(val) => {
                let v = val.clone();
                self.advance()?;
                Expr {
                    kind: ExprKind::Lit(Lit::Str(v)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Bool(val) => {
                let v = *val;
                self.advance()?;
                Expr {
                    kind: ExprKind::Lit(Lit::Bool(v)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::LParen => {
                self.advance()?;
                let inner = self.parse_expr()?;
                self.expect(TokenKind::RParen)?;
                inner
            }
            TokenKind::LBrace => {
                // Block expression
                let block = self.parse_block()?;
                let span = block.span;
                Expr {
                    kind: ExprKind::Block(block),
                    span,
                }
            }
            TokenKind::If => {
                self.advance()?;
                let cond = self.parse_expr()?;
                let then_branch = self.parse_block()?;
                let mut else_branch = None;
                if self.check(&TokenKind::Else) {
                    self.advance()?;
                    if self.check(&TokenKind::If) {
                        // Else-if
                        let else_if = self.parse_expr()?; // Parse expression
                        if let ExprKind::If(c, t, e) = else_if.kind {
                            else_branch = Some(BlockOrIf::If(c, t, e.map(Box::new)));
                        } else {
                            return Err("Expected if expression after else".to_string());
                        }
                    } else {
                        else_branch = Some(BlockOrIf::Block(self.parse_block()?));
                    }
                }
                Expr {
                    kind: ExprKind::If(Box::new(cond), then_branch, else_branch),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Match => {
                self.advance()?;
                let cond = self.parse_expr()?;
                self.expect(TokenKind::LBrace)?;
                let mut arms = Vec::new();
                while !self.check(&TokenKind::RBrace) && !self.check(&TokenKind::EOF) {
                    let arm_span = self.current_token.span;
                    let pattern = self.parse_pattern()?;
                    self.expect(TokenKind::FatArrow)?;
                    let body = self.parse_expr()?;
                    
                    if self.check(&TokenKind::Comma) {
                        self.advance()?;
                    }

                    arms.push(MatchArm {
                        pattern,
                        body,
                        span: arm_span,
                    });
                }
                self.expect(TokenKind::RBrace)?;
                Expr {
                    kind: ExprKind::Match(Box::new(cond), arms),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Amp => {
                // Address-of/borrow Prefix
                self.advance()?;
                let operand = self.parse_expr_bp(70)?; // high precedence prefix
                Expr {
                    kind: ExprKind::Unary(UnOp::AddrOf, Box::new(operand)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Star => {
                // Dereference Prefix
                self.advance()?;
                let operand = self.parse_expr_bp(70)?;
                Expr {
                    kind: ExprKind::Unary(UnOp::Deref, Box::new(operand)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            TokenKind::Bang => {
                self.advance()?;
                let operand = self.parse_expr_bp(70)?;
                Expr {
                    kind: ExprKind::Unary(UnOp::Not, Box::new(operand)),
                    span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                }
            }
            _ => {
                let span = self.current_token.span;
                let msg = format!("Unexpected expression prefix {:?}", self.current_token.kind);
                self.error("E0004", msg, span);
                return Err("Expression parsing error".to_string());
            }
        };

        // Left Denotation (Infix/Postfix)
        loop {
            let op = &self.current_token.kind;
            let (l_bp, r_bp) = match op {
                TokenKind::Plus | TokenKind::Minus => (50, 51),
                TokenKind::Star | TokenKind::Slash => (60, 61),
                TokenKind::EqEq | TokenKind::Ne | TokenKind::Lt | TokenKind::Gt | TokenKind::Le | TokenKind::Ge => (40, 41),
                TokenKind::AmpAmp => (30, 31),
                TokenKind::PipePipe => (20, 21),
                TokenKind::TryOp => (80, 0),
                TokenKind::LParen => (90, 0),
                TokenKind::Dot => (100, 0),
                TokenKind::LBracket => (100, 0),
                _ => break,
            };

            if l_bp < min_bp {
                break;
            }

            match &self.current_token.kind {
                TokenKind::Plus
                | TokenKind::Minus
                | TokenKind::Star
                | TokenKind::Slash
                | TokenKind::EqEq
                | TokenKind::Ne
                | TokenKind::Lt
                | TokenKind::Gt
                | TokenKind::Le
                | TokenKind::Ge
                | TokenKind::AmpAmp
                | TokenKind::PipePipe => {
                    let binop = match &self.current_token.kind {
                        TokenKind::Plus => BinOp::Add,
                        TokenKind::Minus => BinOp::Sub,
                        TokenKind::Star => BinOp::Mul,
                        TokenKind::Slash => BinOp::Div,
                        TokenKind::EqEq => BinOp::Eq,
                        TokenKind::Ne => BinOp::Ne,
                        TokenKind::Lt => BinOp::Lt,
                        TokenKind::Gt => BinOp::Gt,
                        TokenKind::Le => BinOp::Le,
                        TokenKind::Ge => BinOp::Ge,
                        TokenKind::AmpAmp => BinOp::And,
                        TokenKind::PipePipe => BinOp::Or,
                        _ => unreachable!(),
                    };
                    self.advance()?;
                    let rhs = self.parse_expr_bp(r_bp)?;
                    lhs = Expr {
                        kind: ExprKind::Binary(binop, Box::new(lhs), Box::new(rhs)),
                        span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                    };
                }
                TokenKind::TryOp => {
                    self.advance()?;
                    lhs = Expr {
                        kind: ExprKind::Try(Box::new(lhs)),
                        span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                    };
                }
                TokenKind::LParen => {
                    self.advance()?;
                    let mut args = Vec::new();
                    if !self.check(&TokenKind::RParen) {
                        loop {
                            args.push(self.parse_expr()?);
                            if self.check(&TokenKind::Comma) {
                                self.advance()?;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    lhs = Expr {
                        kind: ExprKind::Call(Box::new(lhs), args),
                        span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                    };
                }
                TokenKind::Dot => {
                    self.advance()?;
                    let field_ident = self.expect(TokenKind::Ident(String::new()))?;
                    let field_name = match field_ident.kind {
                        TokenKind::Ident(n) => n,
                        _ => unreachable!(),
                    };

                    if self.check(&TokenKind::LParen) {
                        self.advance()?;
                        let mut args = Vec::new();
                        if !self.check(&TokenKind::RParen) {
                            loop {
                                args.push(self.parse_expr()?);
                                if self.check(&TokenKind::Comma) {
                                    self.advance()?;
                                } else {
                                    break;
                                }
                            }
                        }
                        self.expect(TokenKind::RParen)?;
                        lhs = Expr {
                            kind: ExprKind::MethodCall(Box::new(lhs), field_name, args),
                            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                        };
                    } else {
                        lhs = Expr {
                            kind: ExprKind::FieldAccess(Box::new(lhs), field_name),
                            span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                        };
                    }
                }
                TokenKind::LBracket => {
                    self.advance()?;
                    let index_expr = self.parse_expr()?;
                    self.expect(TokenKind::RBracket)?;
                    lhs = Expr {
                        kind: ExprKind::Index(Box::new(lhs), Box::new(index_expr)),
                        span: Span::new(start_span.start_line, start_span.start_col, self.line, self.col),
                    };
                }
                _ => break,
            }
        }

        Ok(lhs)
    }

    fn parse_pattern(&mut self) -> Result<Pattern, String> {
        let first_tok = &self.current_token.kind;
        match first_tok {
            TokenKind::Ident(n) => {
                let name = n.clone();
                self.advance()?;
                if self.check(&TokenKind::LParen) {
                    // Enum variant with args
                    self.advance()?;
                    let mut args = Vec::new();
                    loop {
                        args.push(self.parse_pattern()?);
                        if self.check(&TokenKind::Comma) {
                            self.advance()?;
                        } else {
                            break;
                        }
                    }
                    self.expect(TokenKind::RParen)?;
                    Ok(Pattern::EnumVariant(vec![name], args))
                } else {
                    Ok(Pattern::Ident(name))
                }
            }
            TokenKind::Int(v, s) => {
                let val = *v;
                let suf = s.clone();
                self.advance()?;
                Ok(Pattern::Lit(Lit::Int(val, suf)))
            }
            TokenKind::Str(v) => {
                let val = v.clone();
                self.advance()?;
                Ok(Pattern::Lit(Lit::Str(val)))
            }
            TokenKind::Bool(v) => {
                let val = *v;
                self.advance()?;
                Ok(Pattern::Lit(Lit::Bool(val)))
            }
            _ => {
                // Check underscore/wildcard
                // Or struct pattern
                let span = self.current_token.span;
                let msg = format!("Unexpected pattern {:?}", self.current_token.kind);
                self.error("E0005", msg, span);
                self.advance()?;
                Err("Pattern parsing error".to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_struct_and_fn() {
        let src = "
        module geometry;
        pub struct Point {
            x: f64,
            y: f64
        }
        impl Point {
            pub fn new(x: f64, y: f64) -> Point {
                Point { x: x, y: y }
            }
        }
        ";
        let lexer = Lexer::new(src, "geometry.cx");
        let mut parser = Parser::new(lexer, "geometry.cx").unwrap();
        let module = parser.parse_module().unwrap();
        
        if !parser.diagnostics.is_empty() {
            println!("Diagnostics: {:#?}", parser.diagnostics);
        }
        
        assert_eq!(module.name, Some("geometry".to_string()));
        assert_eq!(module.items.len(), 2);
        
        if let Item::Struct(s) = &module.items[0] {
            assert_eq!(s.name, "Point");
            assert_eq!(s.fields.len(), 2);
        } else {
            panic!("Expected struct as first item");
        }
    }

    #[test]
    fn test_parser_error_recovery() {
        // Source has three independent syntax errors wrapped in a function
        let src = "
        fn main() {
            let a = ; // syntax error 1
            let b = 10;
            let c = +; // syntax error 2
            let d = 20;
            let e = ; // syntax error 3
        }
        ";
        let mut lexer = Lexer::new(src, "recovery.cx");
        println!("Tokens: {:?}", lexer.lex_all());

        let lexer = Lexer::new(src, "recovery.cx");
        let mut parser = Parser::new(lexer, "recovery.cx").unwrap();
        let _ = parser.parse_module();
        
        if parser.diagnostics.len() != 3 {
            println!("Diagnostics: {:#?}", parser.diagnostics);
        }
        
        // Assert we caught 3 distinct syntax errors
        assert_eq!(parser.diagnostics.len(), 3);
        assert_eq!(parser.diagnostics[0].code, "E0004");
    }
}


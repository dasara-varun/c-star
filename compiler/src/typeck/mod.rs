use crate::ast::*;
use crate::diagnostics::{Diagnostic, Note, Span};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VarState {
    Uninitialized,
    Initialized,
    Moved(Span),
}

#[derive(Debug, Clone, PartialEq)]
pub struct BorrowInfo {
    pub is_mut: bool,
    pub span: Span,
}

pub struct TypeChecker {
    structs: HashMap<String, StructDecl>,
    functions: HashMap<String, FnDecl>,
    // Stack of scopes. Each scope maps variable name -> (Type, VarState, is_mut)
    scopes: Vec<HashMap<String, (Type, VarState, bool)>>,
    // Stack of active borrows.
    active_borrows: Vec<HashMap<String, Vec<BorrowInfo>>>,
    pub diagnostics: Vec<Diagnostic>,
    in_raw_block: bool,
    current_ret_ty: Option<Type>,
    filename: String,
    pub specialized_items: Vec<Item>,
    pub specialized_calls: HashMap<Span, String>,
}

impl TypeChecker {
    pub fn new(filename: &str) -> Self {
        Self {
            structs: HashMap::new(),
            functions: HashMap::new(),
            scopes: Vec::new(),
            active_borrows: Vec::new(),
            diagnostics: Vec::new(),
            in_raw_block: false,
            current_ret_ty: None,
            filename: filename.to_string(),
            specialized_items: Vec::new(),
            specialized_calls: HashMap::new(),
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

    fn types_equal(&self, a: &Type, b: &Type) -> bool {
        if self.in_raw_block {
            match (&a.kind, &b.kind) {
                (TypeKind::RawPtr(_), TypeKind::RawPtr(_)) => return true,
                (TypeKind::RawPtr(_), TypeKind::Ref(_)) => return true,
                (TypeKind::Ref(_), TypeKind::RawPtr(_)) => return true,
                _ => {}
            }
        }
        match (&a.kind, &b.kind) {
            (TypeKind::RawPtr(_), TypeKind::Path(path, _)) => {
                if let Some(name) = path.last() {
                    if name == "i32" || name == "i64" || name == "u8" || name == "usize" || name == "any" {
                        return true;
                    }
                }
            }
            (TypeKind::Path(path, _), TypeKind::RawPtr(_)) => {
                if let Some(name) = path.last() {
                    if name == "i32" || name == "i64" || name == "u8" || name == "usize" || name == "any" {
                        return true;
                    }
                }
            }
            _ => {}
        }
        match (&a.kind, &b.kind) {
            (TypeKind::Path(p1, args1), TypeKind::Path(p2, args2)) => {
                p1 == p2 && args1.len() == args2.len() && args1.iter().zip(args2.iter()).all(|(x, y)| self.types_equal(x, y))
            }
            (TypeKind::Ref(r1), TypeKind::Ref(r2)) => self.types_equal(r1, r2),
            (TypeKind::Slice(s1), TypeKind::Slice(s2)) => self.types_equal(s1, s2),
            (TypeKind::RawPtr(p1), TypeKind::RawPtr(p2)) => self.types_equal(p1, p2),
            _ => false,
        }
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
        self.active_borrows.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
        self.active_borrows.pop();
    }

    fn declare_var(&mut self, name: String, ty: Type, state: VarState, is_mut: bool, span: Span) {
        if let Some(scope) = self.scopes.last_mut() {
            if scope.contains_key(&name) {
                self.error(
                    "E0101",
                    format!("redefinition of local variable `{}`", name),
                    span,
                );
            } else {
                scope.insert(name, (ty, state, is_mut));
            }
        }
    }

    fn lookup_var(&self, name: &str) -> Option<&(Type, VarState, bool)> {
        for scope in self.scopes.iter().rev() {
            if let Some(var) = scope.get(name) {
                return Some(var);
            }
        }
        None
    }

    fn lookup_var_mut(&mut self, name: &str) -> Option<&mut (Type, VarState, bool)> {
        for scope in self.scopes.iter_mut().rev() {
            if let Some(var) = scope.get_mut(name) {
                return Some(var);
            }
        }
        None
    }

    fn check_borrow(&mut self, name: &str, span: Span) {
        let is_mut = if let Some(var) = self.lookup_var(name) {
            var.2
        } else {
            return;
        };

        for scope_borrows in self.active_borrows.iter() {
            if let Some(borrows) = scope_borrows.get(name) {
                for b in borrows {
                    if is_mut {
                        self.error(
                            "E0204",
                            format!("cannot borrow `{}` mutably because it is already borrowed", name),
                            span,
                        );
                        return;
                    } else {
                        if b.is_mut {
                            self.error(
                                "E0204",
                                format!("cannot borrow `{}` immutably because it is already borrowed mutably", name),
                                span,
                            );
                            return;
                        }
                    }
                }
            }
        }

        if let Some(scope_borrows) = self.active_borrows.last_mut() {
            scope_borrows.entry(name.to_string())
                .or_default()
                .push(BorrowInfo { is_mut, span });
        }
    }

    fn is_copy_type(&self, ty: &Type) -> bool {
        match &ty.kind {
            TypeKind::Ref(_) => true,
            TypeKind::RawPtr(_) => true,
            TypeKind::Path(path, _) => {
                if path.len() == 1 {
                    let name = &path[0];
                    matches!(name.as_str(), 
                        "i8" | "i16" | "i32" | "i64" | 
                        "u8" | "u16" | "u32" | "u64" | 
                        "f32" | "f64" | "bool" | "char" | "usize" | "string"
                    )
                } else {
                    false
                }
            }
            TypeKind::Slice(_) => false,
        }
    }

    fn mark_moves(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some((ty, state, _)) = self.lookup_var(name).cloned() {
                    if state == VarState::Initialized && !self.is_copy_type(&ty) {
                        if let Some(var) = self.lookup_var_mut(name) {
                            var.1 = VarState::Moved(expr.span);
                        }
                    }
                }
            }
            ExprKind::Unary(UnOp::AddrOf, _) => {
                // Borrow reference does NOT move
            }
            ExprKind::Unary(_, operand) => {
                self.mark_moves(operand);
            }
            ExprKind::Binary(_, lhs, rhs) => {
                self.mark_moves(lhs);
                self.mark_moves(rhs);
            }
            ExprKind::Call(_, args) => {
                // If it's a function call, we don't move the function path itself, but its arguments
                for arg in args {
                    self.mark_moves(arg);
                }
            }
            ExprKind::MethodCall(receiver, _, args) => {
                self.mark_moves(receiver);
                for arg in args {
                    self.mark_moves(arg);
                }
            }
            ExprKind::FieldAccess(_, _) => {
                // Accessing a field does not move the base struct
            }
            ExprKind::StructInit(_, fields) => {
                for (_, field_expr) in fields {
                    self.mark_moves(field_expr);
                }
            }
            ExprKind::If(cond, _, _) => {
                self.mark_moves(cond);
            }
            ExprKind::Match(cond, _) => {
                self.mark_moves(cond);
            }
            ExprKind::Try(val) => {
                self.mark_moves(val);
            }
            _ => {}
        }
    }

    pub fn check_module(&mut self, module: &Module) {
        // First pass: gather struct definitions and function signatures
        for item in &module.items {
            match item {
                Item::Struct(s) => {
                    self.structs.insert(s.name.clone(), s.clone());
                }
                Item::Fn(f) => {
                    self.functions.insert(f.name.clone(), f.clone());
                }
                Item::Impl(imp) => {
                    // Register impl methods in a flat way for simplified lookup
                    if let TypeKind::Path(path, _) = &imp.target_ty.kind {
                        if let Some(struct_name) = path.last() {
                            for method in &imp.methods {
                                let key = format!("{}::{}", struct_name, method.name);
                                self.functions.insert(key, method.clone());
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Second pass: check all item bodies
        for item in &module.items {
            match item {
                Item::Fn(f) => {
                    if f.generic_params.is_empty() {
                        self.check_fn(f);
                    }
                }
                Item::Impl(imp) => {
                    for method in &imp.methods {
                        if method.generic_params.is_empty() {
                            self.check_fn_with_self(method, Some(imp.target_ty.clone()));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn check_fn(&mut self, f: &FnDecl) {
        self.check_fn_with_self(f, None);
    }

    fn check_fn_with_self(&mut self, f: &FnDecl, self_ty: Option<Type>) {
        self.push_scope();
        let prev_ret = self.current_ret_ty.clone();
        self.current_ret_ty = f.ret_ty.clone();
        if let Some(ref expected_ret) = self.current_ret_ty {
            if let Some(ref self_t) = self_ty {
                self.current_ret_ty = Some(self.resolve_self_type(expected_ret, self_t));
            }
        }

        for param in &f.params {
            let mut ty = param.ty.clone();
            if let Some(ref self_t) = self_ty {
                ty = self.resolve_self_type(&ty, self_t);
            }
            self.declare_var(param.name.clone(), ty, VarState::Initialized, false, param.span);
        }

        self.check_block(&f.body);

        self.pop_scope();
        self.current_ret_ty = prev_ret;
    }

    fn resolve_self_type(&self, ty: &Type, self_ty: &Type) -> Type {
        match &ty.kind {
            TypeKind::Path(path, args) => {
                if path.len() == 1 && (path[0] == "Self" || path[0] == "self") {
                    self_ty.clone()
                } else {
                    let resolved_args = args.iter().map(|arg| self.resolve_self_type(arg, self_ty)).collect();
                    Type {
                        kind: TypeKind::Path(path.clone(), resolved_args),
                        span: ty.span,
                    }
                }
            }
            TypeKind::Ref(inner) => Type {
                kind: TypeKind::Ref(Box::new(self.resolve_self_type(inner, self_ty))),
                span: ty.span,
            },
            TypeKind::Slice(inner) => Type {
                kind: TypeKind::Slice(Box::new(self.resolve_self_type(inner, self_ty))),
                span: ty.span,
            },
            TypeKind::RawPtr(inner) => Type {
                kind: TypeKind::RawPtr(Box::new(self.resolve_self_type(inner, self_ty))),
                span: ty.span,
            },
        }
    }

    fn check_block(&mut self, block: &Block) {
        self.push_scope();
        for stmt in &block.stmts {
            self.check_stmt(stmt);
        }
        self.pop_scope();
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match &stmt.kind {
            StmtKind::Let { is_mut, name, ty, value } => {
                let val_ty = match self.check_expr(value) {
                    Some(t) => t,
                    None => return,
                };
                self.mark_moves(value);

                if let Some(expected_ty) = ty {
                    if !self.types_equal(&val_ty, expected_ty) {
                        self.error(
                            "E0102",
                            format!(
                                "mismatched types: expected `{:?}`, found `{:?}`",
                                expected_ty.kind, val_ty.kind
                            ),
                            value.span,
                        );
                    }
                    self.declare_var(name.clone(), expected_ty.clone(), VarState::Initialized, *is_mut, stmt.span);
                } else {
                    self.declare_var(name.clone(), val_ty, VarState::Initialized, *is_mut, stmt.span);
                }
            }
            StmtKind::Assign(lhs, rhs) => {
                let lhs_ty = match self.check_expr(lhs) {
                    Some(t) => t,
                    None => return,
                };
                let rhs_ty = match self.check_expr(rhs) {
                    Some(t) => t,
                    None => return,
                };
                self.mark_moves(rhs);

                if !self.types_equal(&lhs_ty, &rhs_ty) {
                    self.error(
                        "E0103",
                        format!(
                            "mismatched types in assignment: expected `{:?}`, found `{:?}`",
                            lhs_ty.kind, rhs_ty.kind
                        ),
                        rhs.span,
                    );
                }

                // If lhs is an identifier, mark it as initialized
                if let ExprKind::Ident(ref name) = lhs.kind {
                    if let Some(var) = self.lookup_var_mut(name) {
                        var.1 = VarState::Initialized;
                    }
                }
            }
            StmtKind::Expr(expr) => {
                self.check_expr(expr);
                self.mark_moves(expr);
            }
            StmtKind::Return(val_expr) => {
                let actual_ty = match val_expr {
                    Some(expr) => {
                        let ty = self.check_expr(expr);
                        self.mark_moves(expr);
                        ty
                    }
                    None => None,
                };

                match (&self.current_ret_ty, actual_ty) {
                    (Some(expected), Some(actual)) => {
                        if !self.types_equal(expected, &actual) {
                            self.error(
                                "E0104",
                                format!(
                                    "mismatched return type: expected `{:?}`, found `{:?}`",
                                    expected.kind, actual.kind
                                ),
                                val_expr.as_ref().unwrap().span,
                            );
                        }
                    }
                    (None, Some(actual)) => {
                        // Expected void/no return type
                        self.error(
                            "E0105",
                            format!("mismatched return type: expected void, found `{:?}`", actual.kind),
                            val_expr.as_ref().unwrap().span,
                        );
                    }
                    (Some(expected), None) => {
                        self.error(
                            "E0106",
                            format!("mismatched return type: expected `{:?}`, found void", expected.kind),
                            stmt.span,
                        );
                    }
                    (None, None) => {}
                }
            }
            StmtKind::While(cond, body) => {
                let cond_ty = self.check_expr(cond);
                self.mark_moves(cond);
                if let Some(t) = cond_ty {
                    let bool_ty = Type {
                        kind: TypeKind::Path(vec!["bool".to_string()], Vec::new()),
                        span: cond.span,
                    };
                    if !self.types_equal(&t, &bool_ty) {
                        self.error(
                            "E0107",
                            format!("while condition must be bool, found `{:?}`", t.kind),
                            cond.span,
                        );
                    }
                }
                self.check_block(body);
            }
            StmtKind::For(name, iter, body) => {
                let _iter_ty = self.check_expr(iter); // simplified typecheck for for loop iterator
                self.push_scope();
                let dummy_ty = Type {
                    kind: TypeKind::Path(vec!["i32".to_string()], Vec::new()),
                    span: stmt.span,
                };
                self.declare_var(name.clone(), dummy_ty, VarState::Initialized, false, stmt.span);
                self.check_block(body);
                self.pop_scope();
            }
            StmtKind::Raw(block) => {
                let prev_raw = self.in_raw_block;
                self.in_raw_block = true;
                self.check_block(block);
                self.in_raw_block = prev_raw;
            }
        }
    }

    fn check_expr(&mut self, expr: &Expr) -> Option<Type> {
        match &expr.kind {
            ExprKind::Lit(lit) => match lit {
                Lit::Int(_, suffix) => {
                    let ty_name = suffix.clone().unwrap_or_else(|| "i32".to_string());
                    Some(Type {
                        kind: TypeKind::Path(vec![ty_name], Vec::new()),
                        span: expr.span,
                    })
                }
                Lit::Float(_, suffix) => {
                    let ty_name = suffix.clone().unwrap_or_else(|| "f64".to_string());
                    Some(Type {
                        kind: TypeKind::Path(vec![ty_name], Vec::new()),
                        span: expr.span,
                    })
                }
                Lit::Str(_) => Some(Type {
                    kind: TypeKind::Path(vec!["string".to_string()], Vec::new()),
                    span: expr.span,
                }),
                Lit::Bool(_) => Some(Type {
                    kind: TypeKind::Path(vec!["bool".to_string()], Vec::new()),
                    span: expr.span,
                }),
            },
            ExprKind::Ident(name) => {
                if let Some((ty, state, _)) = self.lookup_var(name).cloned() {
                    match state {
                        VarState::Uninitialized => {
                            self.error(
                                "E0108",
                                format!("use of possibly uninitialized variable `{}`", name),
                                expr.span,
                            );
                        }
                        VarState::Moved(move_span) => {
                            self.diagnostics.push(Diagnostic {
                                code: "E0203".to_string(),
                                severity: "error".to_string(),
                                message: format!("use of moved value `{}`", name),
                                file: self.filename.clone(),
                                span: expr.span,
                                notes: vec![Note {
                                    message: format!("`{}` was moved here", name),
                                    span: Some(move_span),
                                }],
                                suggested_fix: None,
                            });
                        }
                        VarState::Initialized => {}
                    }
                    Some(ty)
                } else if self.functions.contains_key(name) {
                    Some(Type {
                        kind: TypeKind::RawPtr(Box::new(Type {
                            kind: TypeKind::Path(vec!["u8".to_string()], Vec::new()),
                            span: expr.span,
                        })),
                        span: expr.span,
                    })
                } else {
                    self.error(
                        "E0109",
                        format!("cannot find value `{}` in this scope", name),
                        expr.span,
                    );
                    None
                }
            }
            ExprKind::Path(path) => {
                // Simplified resolution for paths (e.g. Option::None)
                let path_str = path.join("::");
                Some(Type {
                    kind: TypeKind::Path(vec![path_str], Vec::new()),
                    span: expr.span,
                })
            }
            ExprKind::Binary(op, lhs, rhs) => {
                let lhs_ty = self.check_expr(lhs)?;
                let rhs_ty = self.check_expr(rhs)?;

                if !self.types_equal(&lhs_ty, &rhs_ty) {
                    self.error(
                        "E0110",
                        format!(
                            "mismatched types in binary operation: `{:?}` and `{:?}`",
                            lhs_ty.kind, rhs_ty.kind
                        ),
                        expr.span,
                    );
                    return None;
                }

                match op {
                    BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => Some(lhs_ty),
                    BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => Some(Type {
                        kind: TypeKind::Path(vec!["bool".to_string()], Vec::new()),
                        span: expr.span,
                    }),
                    BinOp::And | BinOp::Or => Some(lhs_ty),
                }
            }
            ExprKind::Unary(op, operand) => {
                let ty = self.check_expr(operand)?;
                match op {
                    UnOp::AddrOf => {
                        if let ExprKind::Ident(ref name) = operand.kind {
                            self.check_borrow(name, expr.span);
                        }
                        Some(Type {
                            kind: TypeKind::Ref(Box::new(ty)),
                            span: expr.span,
                        })
                    }
                    UnOp::Deref => {
                        // Dereference is only valid for references (&T) or raw pointers (raw *T)
                        match ty.kind {
                            TypeKind::Ref(inner) => Some(*inner),
                            TypeKind::RawPtr(inner) => {
                                // Raw pointer dereference requires raw block!
                                if !self.in_raw_block {
                                    self.error(
                                        "E0111",
                                        "raw pointer dereference requires `raw` block".to_string(),
                                        expr.span,
                                    );
                                }
                                Some(*inner)
                            }
                            _ => {
                                self.error(
                                    "E0112",
                                    format!("cannot dereference type `{:?}`", ty.kind),
                                    expr.span,
                                );
                                None
                            }
                        }
                    }
                    UnOp::Not => Some(ty),
                }
            }
            ExprKind::FieldAccess(receiver, field_name) => {
                let receiver_ty = self.check_expr(receiver)?;
                // Handle struct field access
                let struct_name = match &receiver_ty.kind {
                    TypeKind::Path(path, _) => path.last()?.clone(),
                    TypeKind::Ref(inner) => match &inner.kind {
                        TypeKind::Path(path, _) => path.last()?.clone(),
                        _ => {
                            self.error("E0113", "receiver is not a struct".to_string(), expr.span);
                            return None;
                        }
                    },
                    _ => {
                        self.error("E0113", "receiver is not a struct".to_string(), expr.span);
                        return None;
                    }
                };

                if let Some(s) = self.structs.get(&struct_name) {
                    if let Some(f) = s.fields.iter().find(|f| &f.name == field_name) {
                        return Some(f.ty.clone());
                    }
                }

                self.error(
                    "E0114",
                    format!("no field `{}` on type `{}`", field_name, struct_name),
                    expr.span,
                );
                None
            }
            ExprKind::MethodCall(receiver, method_name, args) => {
                let receiver_ty = self.check_expr(receiver)?;
                let struct_name = match &receiver_ty.kind {
                    TypeKind::Path(path, _) => path.last()?.clone(),
                    TypeKind::Ref(inner) => match &inner.kind {
                        TypeKind::Path(path, _) => path.last()?.clone(),
                        _ => {
                            self.error("E0115", "receiver is not a struct".to_string(), expr.span);
                            return None;
                        }
                    },
                    _ => {
                        self.error("E0115", "receiver is not a struct".to_string(), expr.span);
                        return None;
                    }
                };

                let key = format!("{}::{}", struct_name, method_name);
                if let Some(method) = self.functions.get(&key).cloned() {
                    // Check method arguments (excluding self)
                    // Method parameters have self as first arg
                    let expected_params = &method.params;
                    if expected_params.is_empty() || expected_params[0].name != "self" {
                        self.error(
                            "E0116",
                            format!("method `{}` has no `self` parameter", method_name),
                            expr.span,
                        );
                        return None;
                    }

                    if args.len() != expected_params.len() - 1 {
                        self.error(
                            "E0117",
                            format!(
                                "method `{}` expected {} arguments, found {}",
                                method_name,
                                expected_params.len() - 1,
                                args.len()
                            ),
                            expr.span,
                        );
                        return None;
                    }

                    for (i, arg) in args.iter().enumerate() {
                        let arg_ty = self.check_expr(arg)?;
                        let param_ty = &expected_params[i + 1].ty;
                        if !self.types_equal(&arg_ty, param_ty) {
                            self.error(
                                "E0118",
                                format!(
                                    "mismatched types in method call: expected `{:?}`, found `{:?}`",
                                    param_ty.kind, arg_ty.kind
                                ),
                                arg.span,
                            );
                        }
                    }

                    return method.ret_ty.clone();
                }

                self.error(
                    "E0119",
                    format!("no method `{}` on type `{}`", method_name, struct_name),
                    expr.span,
                );
                None
            }
            ExprKind::Call(callee, args) => {
                // If callee is identifier/path, look up function
                let func_name = match &callee.kind {
                    ExprKind::Ident(name) => name.clone(),
                    ExprKind::Path(path) => path.join("::"),
                    _ => {
                        self.error("E0120", "callee is not a function".to_string(), callee.span);
                        return None;
                    }
                };

                // In safe code, prevent passing references to async spawn function
                if func_name == "spawn" && !self.in_raw_block {
                    for arg in args {
                        if let Some(arg_ty) = self.check_expr(arg) {
                            if let TypeKind::Ref(_) = arg_ty.kind {
                                self.error(
                                    "E0205",
                                    "cannot pass reference as thread argument in safe code".to_string(),
                                    arg.span,
                                );
                            }
                        }
                    }
                }

                // Stub check for standard library functions
                if func_name == "print" || func_name == "sqrt" {
                    for arg in args {
                        self.check_expr(arg);
                    }
                    if func_name == "sqrt" {
                        return Some(Type {
                            kind: TypeKind::Path(vec!["f64".to_string()], Vec::new()),
                            span: expr.span,
                        });
                    }
                    return None; // print returns void/none
                }

                if func_name == "Ok" || func_name == "Err" || func_name == "Some" || func_name == "None" {
                    for arg in args {
                        self.check_expr(arg);
                    }
                    return self.current_ret_ty.clone();
                }

                if let Some(func) = self.functions.get(&func_name).cloned() {
                    let active_func = if !func.generic_params.is_empty() {
                        let mut arg_tys = Vec::new();
                        for arg in args {
                            if let Some(t) = self.check_expr(arg) {
                                arg_tys.push(t);
                            } else {
                                return None;
                            }
                        }
                        let param_tys: Vec<Type> = func.params.iter().map(|p| p.ty.clone()).collect();
                        let concrete_args = self.infer_generic_args(&func.generic_params, &param_tys, &arg_tys)?;
                        let spec_f = self.specialize_fn(&func, &concrete_args);
                        
                        let spec_name = spec_f.name.clone();
                        self.specialized_calls.insert(expr.span, spec_name.clone());

                        if !self.functions.contains_key(&spec_name) {
                            self.functions.insert(spec_name.clone(), spec_f.clone());
                            self.specialized_items.push(Item::Fn(spec_f.clone()));
                            self.check_fn(&spec_f);
                        }
                        self.functions.get(&spec_name).cloned()?
                    } else {
                        func
                    };

                    if args.len() != active_func.params.len() {
                        self.error(
                            "E0121",
                            format!(
                                "function `{}` expected {} arguments, found {}",
                                active_func.name,
                                active_func.params.len(),
                                args.len()
                            ),
                            expr.span,
                        );
                        return None;
                    }

                    for (i, arg) in args.iter().enumerate() {
                        let arg_ty = self.check_expr(arg)?;
                        let param_ty = &active_func.params[i].ty;
                        if !self.types_equal(&arg_ty, param_ty) {
                            self.error(
                                "E0122",
                                format!(
                                    "mismatched types in function call: expected `{:?}`, found `{:?}`",
                                    param_ty.kind, arg_ty.kind
                                ),
                                arg.span,
                            );
                        }
                    }

                    return active_func.ret_ty.clone();
                }

                self.error(
                    "E0123",
                    format!("cannot find function `{}` in this scope", func_name),
                    expr.span,
                );
                None
            }
            ExprKind::StructInit(path, fields) => {
                let struct_name = path.last()?;
                if let Some(s) = self.structs.get(struct_name).cloned() {
                    // Check if all fields are initialized with correct types
                    let mut initialized_fields = std::collections::HashSet::new();
                    for (f_name, f_val) in fields {
                        initialized_fields.insert(f_name.clone());
                        let val_ty = self.check_expr(f_val)?;
                        if let Some(expected_field) = s.fields.iter().find(|f| &f.name == f_name) {
                            if !self.types_equal(&val_ty, &expected_field.ty) {
                                self.error(
                                    "E0124",
                                    format!(
                                        "mismatched field type for `{}`: expected `{:?}`, found `{:?}`",
                                        f_name, expected_field.ty.kind, val_ty.kind
                                    ),
                                    f_val.span,
                                );
                            }
                        } else {
                            self.error(
                                "E0125",
                                format!("struct `{}` has no field `{}`", struct_name, f_name),
                                f_val.span,
                            );
                        }
                    }

                    for expected_field in &s.fields {
                        if !initialized_fields.contains(&expected_field.name) {
                            self.error(
                                "E0126",
                                format!("missing field `{}` in struct initializer", expected_field.name),
                                expr.span,
                            );
                        }
                    }

                    Some(Type {
                        kind: TypeKind::Path(path.clone(), Vec::new()),
                        span: expr.span,
                    })
                } else {
                    self.error(
                        "E0127",
                        format!("cannot find struct `{}` in this scope", struct_name),
                        expr.span,
                    );
                    None
                }
            }
            ExprKind::Block(block) => {
                self.push_scope();
                let mut last_ty = None;
                for stmt in &block.stmts {
                    self.check_stmt(stmt);
                    if let StmtKind::Expr(expr) = &stmt.kind {
                        last_ty = self.check_expr(expr);
                    }
                }
                self.pop_scope();
                last_ty
            }
            ExprKind::If(cond, then_branch, else_branch) => {
                let cond_ty = self.check_expr(cond)?;
                let bool_ty = Type {
                    kind: TypeKind::Path(vec!["bool".to_string()], Vec::new()),
                    span: cond.span,
                };
                if !self.types_equal(&cond_ty, &bool_ty) {
                    self.error(
                        "E0128",
                        format!("if condition must be bool, found `{:?}`", cond_ty.kind),
                        cond.span,
                    );
                }

                // Check block branches types
                // If expression must return same types on both branches if it is used as an expression,
                // but for simplicity we just typecheck them.
                self.check_block(then_branch);
                if let Some(eb) = else_branch {
                    match eb {
                        BlockOrIf::Block(b) => self.check_block(b),
                        BlockOrIf::If(c, t, e) => {
                            // Recursively check else-if
                            let dummy_expr = Expr {
                                kind: ExprKind::If(c.clone(), t.clone(), e.clone().map(|x| *x)),
                                span: expr.span,
                            };
                            self.check_expr(&dummy_expr);
                        }
                    }
                }
                None // simplified to None for v0.1 statement-like if
            }
            ExprKind::Match(cond, arms) => {
                let _cond_ty = self.check_expr(cond)?;
                for arm in arms {
                    // Check pattern binding, etc.
                    self.push_scope();
                    // Bind pattern variables
                    self.bind_pattern(&arm.pattern);
                    self.check_expr(&arm.body);
                    self.pop_scope();
                }
                None
            }
            ExprKind::Try(val) => {
                let ty = self.check_expr(val)?;
                // expr? unwraps Result<T, E> to T, or Option<T> to T
                match ty.kind {
                    TypeKind::Path(path, args) => {
                        if path.last()? == "Result" && args.len() == 2 {
                            Some(args[0].clone())
                        } else if path.last()? == "Option" && args.len() == 1 {
                            Some(args[0].clone())
                        } else {
                            self.error(
                                "E0129",
                                format!("operator ? cannot be applied to type `{:?}`", TypeKind::Path(path, args)),
                                expr.span,
                            );
                            None
                        }
                    }
                    _ => {
                        self.error(
                            "E0129",
                            format!("operator ? cannot be applied to type `{:?}`", ty.kind),
                            expr.span,
                        );
                        None
                    }
                }
            }
            ExprKind::Index(receiver, index) => {
                let rx_ty = self.check_expr(receiver)?;
                let idx_ty = self.check_expr(index)?;

                let elem_ty = match rx_ty.kind {
                    TypeKind::Slice(inner) => *inner,
                    _ => {
                        self.error(
                            "E0130",
                            format!("cannot index into non-slice type `{:?}`", rx_ty.kind),
                            receiver.span,
                        );
                        return None;
                    }
                };

                let is_integer = match &idx_ty.kind {
                    TypeKind::Path(path, _) if path.len() == 1 => {
                        let name = &path[0];
                        matches!(name.as_str(), "i32" | "i64" | "usize" | "u32" | "u64" | "i16" | "u16" | "i8" | "u8")
                    }
                    _ => false,
                };

                if !is_integer {
                    self.error(
                        "E0131",
                        format!("slice index must be an integer, found `{:?}`", idx_ty.kind),
                        index.span,
                    );
                }

                Some(elem_ty)
            }
        }
    }

    fn substitute_type(&self, ty: &Type, generic_params: &[String], generic_args: &[Type]) -> Type {
        let kind = match &ty.kind {
            TypeKind::Path(path, args) => {
                if path.len() == 1 {
                    let name = &path[0];
                    if let Some(idx) = generic_params.iter().position(|p| p == name) {
                        return generic_args[idx].clone();
                    }
                }
                let sub_args = args.iter().map(|arg| self.substitute_type(arg, generic_params, generic_args)).collect();
                TypeKind::Path(path.clone(), sub_args)
            }
            TypeKind::Ref(inner) => {
                TypeKind::Ref(Box::new(self.substitute_type(inner, generic_params, generic_args)))
            }
            TypeKind::Slice(inner) => {
                TypeKind::Slice(Box::new(self.substitute_type(inner, generic_params, generic_args)))
            }
            TypeKind::RawPtr(inner) => {
                TypeKind::RawPtr(Box::new(self.substitute_type(inner, generic_params, generic_args)))
            }
        };
        Type {
            kind,
            span: ty.span,
        }
    }

    fn substitute_pattern(&self, pat: &Pattern, generic_params: &[String], generic_args: &[Type]) -> Pattern {
        match pat {
            Pattern::Ident(_) | Pattern::Lit(_) | Pattern::Underscore => pat.clone(),
            Pattern::Struct(path, fields) => {
                let sub_fields = fields.iter().map(|(name, p)| (name.clone(), self.substitute_pattern(p, generic_params, generic_args))).collect();
                Pattern::Struct(path.clone(), sub_fields)
            }
            Pattern::EnumVariant(path, args) => {
                let sub_args = args.iter().map(|p| self.substitute_pattern(p, generic_params, generic_args)).collect();
                Pattern::EnumVariant(path.clone(), sub_args)
            }
        }
    }

    fn substitute_expr(&self, expr: &Expr, generic_params: &[String], generic_args: &[Type]) -> Expr {
        let kind = match &expr.kind {
            ExprKind::Lit(_) | ExprKind::Ident(_) | ExprKind::Path(_) => expr.kind.clone(),
            ExprKind::Binary(op, lhs, rhs) => {
                ExprKind::Binary(*op, Box::new(self.substitute_expr(lhs, generic_params, generic_args)), Box::new(self.substitute_expr(rhs, generic_params, generic_args)))
            }
            ExprKind::Unary(op, operand) => {
                ExprKind::Unary(*op, Box::new(self.substitute_expr(operand, generic_params, generic_args)))
            }
            ExprKind::FieldAccess(receiver, field) => {
                ExprKind::FieldAccess(Box::new(self.substitute_expr(receiver, generic_params, generic_args)), field.clone())
            }
            ExprKind::MethodCall(receiver, method, args) => {
                let sub_args = args.iter().map(|arg| self.substitute_expr(arg, generic_params, generic_args)).collect();
                ExprKind::MethodCall(Box::new(self.substitute_expr(receiver, generic_params, generic_args)), method.clone(), sub_args)
            }
            ExprKind::Call(callee, args) => {
                let sub_args = args.iter().map(|arg| self.substitute_expr(arg, generic_params, generic_args)).collect();
                ExprKind::Call(Box::new(self.substitute_expr(callee, generic_params, generic_args)), sub_args)
            }
            ExprKind::StructInit(path, fields) => {
                let sub_fields = fields.iter().map(|(name, e)| (name.clone(), self.substitute_expr(e, generic_params, generic_args))).collect();
                ExprKind::StructInit(path.clone(), sub_fields)
            }
            ExprKind::Block(block) => {
                ExprKind::Block(self.substitute_block(block, generic_params, generic_args))
            }
            ExprKind::If(cond, then_branch, else_branch) => {
                let sub_else = else_branch.as_ref().map(|eb| match eb {
                    BlockOrIf::Block(b) => BlockOrIf::Block(self.substitute_block(b, generic_params, generic_args)),
                    BlockOrIf::If(c, t, e) => {
                        let sub_c = self.substitute_expr(c, generic_params, generic_args);
                        let sub_t = self.substitute_block(t, generic_params, generic_args);
                        let sub_e = e.as_ref().map(|x| Box::new(match &**x {
                            BlockOrIf::Block(b) => BlockOrIf::Block(self.substitute_block(b, generic_params, generic_args)),
                            BlockOrIf::If(c2, t2, e2) => {
                                let dummy = Expr {
                                    kind: ExprKind::If(c2.clone(), t2.clone(), e2.clone().map(|y| *y)),
                                    span: expr.span,
                                };
                                let sub_dummy = self.substitute_expr(&dummy, generic_params, generic_args);
                                match sub_dummy.kind {
                                    ExprKind::If(c3, t3, Some(e3)) => BlockOrIf::If(c3, t3, Some(Box::new(e3))),
                                    ExprKind::If(c3, t3, None) => BlockOrIf::If(c3, t3, None),
                                    _ => unreachable!(),
                                }
                            }
                        }));
                        BlockOrIf::If(Box::new(sub_c), sub_t, sub_e)
                    }
                });
                ExprKind::If(Box::new(self.substitute_expr(cond, generic_params, generic_args)), self.substitute_block(then_branch, generic_params, generic_args), sub_else)
            }
            ExprKind::Match(cond, arms) => {
                let sub_arms = arms.iter().map(|arm| MatchArm {
                    pattern: self.substitute_pattern(&arm.pattern, generic_params, generic_args),
                    body: self.substitute_expr(&arm.body, generic_params, generic_args),
                    span: arm.span,
                }).collect();
                ExprKind::Match(Box::new(self.substitute_expr(cond, generic_params, generic_args)), sub_arms)
            }
            ExprKind::Try(val) => {
                ExprKind::Try(Box::new(self.substitute_expr(val, generic_params, generic_args)))
            }
            ExprKind::Index(receiver, index) => {
                ExprKind::Index(
                    Box::new(self.substitute_expr(receiver, generic_params, generic_args)),
                    Box::new(self.substitute_expr(index, generic_params, generic_args)),
                )
            }
        };
        Expr {
            kind,
            span: expr.span,
        }
    }

    fn substitute_block(&self, block: &Block, generic_params: &[String], generic_args: &[Type]) -> Block {
        let sub_stmts = block.stmts.iter().map(|stmt| {
            let kind = match &stmt.kind {
                StmtKind::Let { is_mut, name, ty, value } => {
                    let sub_ty = ty.as_ref().map(|t| self.substitute_type(t, generic_params, generic_args));
                    let sub_val = self.substitute_expr(value, generic_params, generic_args);
                    StmtKind::Let { is_mut: *is_mut, name: name.clone(), ty: sub_ty, value: sub_val }
                }
                StmtKind::Assign(lhs, rhs) => {
                    StmtKind::Assign(self.substitute_expr(lhs, generic_params, generic_args), self.substitute_expr(rhs, generic_params, generic_args))
                }
                StmtKind::Expr(expr) => {
                    StmtKind::Expr(self.substitute_expr(expr, generic_params, generic_args))
                }
                StmtKind::Return(val_expr) => {
                    StmtKind::Return(val_expr.as_ref().map(|e| self.substitute_expr(e, generic_params, generic_args)))
                }
                StmtKind::While(cond, body) => {
                    StmtKind::While(self.substitute_expr(cond, generic_params, generic_args), self.substitute_block(body, generic_params, generic_args))
                }
                StmtKind::For(name, iter, body) => {
                    StmtKind::For(name.clone(), self.substitute_expr(iter, generic_params, generic_args), self.substitute_block(body, generic_params, generic_args))
                }
                StmtKind::Raw(body) => {
                    StmtKind::Raw(self.substitute_block(body, generic_params, generic_args))
                }
            };
            Stmt {
                kind,
                span: stmt.span,
            }
        }).collect();

        Block {
            stmts: sub_stmts,
            span: block.span,
        }
    }

    pub fn specialize_fn(&self, f: &FnDecl, generic_args: &[Type]) -> FnDecl {
        let generic_params = &f.generic_params;
        let sub_params = f.params.iter().map(|p| Param {
            name: p.name.clone(),
            ty: self.substitute_type(&p.ty, generic_params, generic_args),
            span: p.span,
        }).collect();
        let sub_ret_ty = f.ret_ty.as_ref().map(|t| self.substitute_type(t, generic_params, generic_args));
        let sub_body = self.substitute_block(&f.body, generic_params, generic_args);
        
        let arg_names: Vec<String> = generic_args.iter().map(|arg| {
            match &arg.kind {
                TypeKind::Path(path, _) => path.last().unwrap().clone(),
                _ => "ptr".to_string(),
            }
        }).collect();
        let specialized_name = format!("{}_{}", f.name, arg_names.join("_"));

        FnDecl {
            is_pub: f.is_pub,
            name: specialized_name,
            generic_params: Vec::new(),
            params: sub_params,
            ret_ty: sub_ret_ty,
            body: sub_body,
            span: f.span,
        }
    }

    fn infer_generic_args(&self, generic_params: &[String], param_tys: &[Type], arg_tys: &[Type]) -> Option<Vec<Type>> {
        let mut inferred = vec![None; generic_params.len()];
        for (param_ty, arg_ty) in param_tys.iter().zip(arg_tys.iter()) {
            self.match_param_arg_type(generic_params, param_ty, arg_ty, &mut inferred);
        }
        
        let mut concrete_args = Vec::new();
        for opt in inferred {
            if let Some(ty) = opt {
                concrete_args.push(ty);
            } else {
                // Fallback to i32
                concrete_args.push(Type {
                    kind: TypeKind::Path(vec!["i32".to_string()], Vec::new()),
                    span: Span::new(0, 0, 0, 0),
                });
            }
        }
        Some(concrete_args)
    }

    fn match_param_arg_type(&self, generic_params: &[String], param_ty: &Type, arg_ty: &Type, inferred: &mut [Option<Type>]) {
        match (&param_ty.kind, &arg_ty.kind) {
            (TypeKind::Path(path, args), TypeKind::Path(_, arg_args)) => {
                if path.len() == 1 {
                    let name = &path[0];
                    if let Some(idx) = generic_params.iter().position(|p| p == name) {
                        if inferred[idx].is_none() {
                            inferred[idx] = Some(arg_ty.clone());
                        }
                        return;
                    }
                }
                for (p_sub, a_sub) in args.iter().zip(arg_args.iter()) {
                    self.match_param_arg_type(generic_params, p_sub, a_sub, inferred);
                }
            }
            (TypeKind::Ref(p_inner), TypeKind::Ref(a_inner)) => {
                self.match_param_arg_type(generic_params, p_inner, a_inner, inferred);
            }
            (TypeKind::Slice(p_inner), TypeKind::Slice(a_inner)) => {
                self.match_param_arg_type(generic_params, p_inner, a_inner, inferred);
            }
            (TypeKind::RawPtr(p_inner), TypeKind::RawPtr(a_inner)) => {
                self.match_param_arg_type(generic_params, p_inner, a_inner, inferred);
            }
            _ => {}
        }
    }

    fn bind_pattern(&mut self, pattern: &Pattern) {
        match pattern {
            Pattern::Ident(name) => {
                let dummy_ty = Type {
                    kind: TypeKind::Path(vec!["any".to_string()], Vec::new()),
                    span: Span::new(0, 0, 0, 0),
                };
                self.declare_var(name.clone(), dummy_ty, VarState::Initialized, false, Span::new(0, 0, 0, 0));
            }
            Pattern::EnumVariant(_, subpatterns) => {
                for pat in subpatterns {
                    self.bind_pattern(pat);
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn check_src(src: &str) -> Vec<Diagnostic> {
        let lexer = Lexer::new(src, "test.cx");
        let mut parser = Parser::new(lexer, "test.cx").unwrap();
        let module = parser.parse_module().unwrap();
        let mut tc = TypeChecker::new("test.cx");
        tc.check_module(&module);
        tc.diagnostics
    }

    #[test]
    fn test_type_mismatch() {
        let src = "
        fn main() {
            let x: i32 = 10;
            let y: f64 = x; // Error: mismatched type
        }
        ";
        let errors = check_src(src);
        assert!(!errors.is_empty());
        assert_eq!(errors[0].code, "E0102");
    }

    #[test]
    fn test_redefinition() {
        let src = "
        fn main() {
            let x: i32 = 10;
            let x: i32 = 20; // Error: redefinition
        }
        ";
        let errors = check_src(src);
        assert!(!errors.is_empty());
        assert_eq!(errors[0].code, "E0101");
    }

    #[test]
    fn test_uninitialized_var() {
        // C* let bindings require initializers in parser, but we can verify our lookup logic.
        // Let's check a standard valid initialization
        let src = "
        fn main() {
            let x: i32 = 10;
            let y: i32 = x + 5;
        }
        ";
        let errors = check_src(src);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_raw_deref_outside_raw_block() {
        // Let's test raw pointer deref error outside of a raw block
        let src = "
        fn main() {
            let ptr: raw *i32 = 0; // stub type/val
            let val: i32 = *ptr;   // Dereferencing raw pointer!
        }
        ";
        // Note: 'raw *i32' parses as TypeKind::RawPtr.
        // UnOp::Deref on it should flag error E0111 because it's not inside a raw block.
        let errors = check_src(src);
        assert!(!errors.is_empty());
        assert!(errors.iter().any(|e| e.code == "E0111"));
    }

    #[test]
    fn test_raw_deref_inside_raw_block() {
        let src = "
        fn main() {
            let ptr: raw *i32 = 0;
            raw {
                let val: i32 = *ptr;
            }
        }
        ";
        let errors = check_src(src);
        // Inside raw block, it should be allowed!
        assert!(errors.is_empty() || !errors.iter().any(|e| e.code == "E0111"));
    }

    #[test]
    fn test_use_after_move() {
        let src = "
        struct Point {
            x: f64,
            y: f64,
        }
        fn consume(p: Point) {}
        fn main() {
            let p1 = Point { x: 0.0, y: 0.0 };
            consume(p1);
            let p2 = p1; // Error: use of moved value p1
        }
        ";
        let errors = check_src(src);
        assert!(!errors.is_empty(), "Expected E0203 typecheck error for use-after-move");
        assert_eq!(errors[0].code, "E0203");
        assert!(errors[0].message.contains("use of moved value `p1`"));
        assert!(errors[0].notes[0].message.contains("`p1` was moved here"));
    }
}

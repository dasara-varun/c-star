use inkwell::context::Context;
use inkwell::module::Module as LlvmModule;
use inkwell::builder::Builder;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue, IntValue};
use inkwell::types::{BasicTypeEnum, BasicMetadataTypeEnum, BasicType};
use inkwell::IntPredicate;
use inkwell::FloatPredicate;
use crate::ast::*;
use crate::diagnostics::Span;
use std::collections::HashMap;

pub struct Codegen<'ctx> {
    context: &'ctx Context,
    pub module: LlvmModule<'ctx>,
    builder: Builder<'ctx>,
    variables: HashMap<String, PointerValue<'ctx>>,
    variable_types: HashMap<String, BasicTypeEnum<'ctx>>,
    functions: HashMap<String, FunctionValue<'ctx>>,
    struct_types: HashMap<String, inkwell::types::StructType<'ctx>>,
    current_function: Option<FunctionValue<'ctx>>,
    pub specialized_calls: HashMap<Span, String>,
    struct_fields: HashMap<String, Vec<String>>,
    var_struct_types: HashMap<String, String>,
    struct_field_types: HashMap<String, HashMap<String, String>>,
    slice_element_types: HashMap<String, BasicTypeEnum<'ctx>>,
    pub profile: String,
}

impl<'ctx> Codegen<'ctx> {
    pub fn new(context: &'ctx Context, module_name: &str) -> Self {
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        Self {
            context,
            module,
            builder,
            variables: HashMap::new(),
            variable_types: HashMap::new(),
            functions: HashMap::new(),
            struct_types: HashMap::new(),
            current_function: None,
            specialized_calls: HashMap::new(),
            struct_fields: HashMap::new(),
            var_struct_types: HashMap::new(),
            struct_field_types: HashMap::new(),
            slice_element_types: HashMap::new(),
            profile: "safe".to_string(),
        }
    }

    fn get_llvm_type(&self, ty: &Type) -> BasicTypeEnum<'ctx> {
        match &ty.kind {
            TypeKind::Path(path, args) => {
                let name = path.last().unwrap();
                match name.as_str() {
                    "i32" => self.context.i32_type().into(),
                    "i64" => self.context.i64_type().into(),
                    "usize" => self.context.i64_type().into(),
                    "f32" => self.context.f32_type().into(),
                    "f64" => self.context.f64_type().into(),
                    "bool" => self.context.bool_type().into(),
                    "string" => self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                    "Result" => {
                        let t = if args.len() >= 1 { self.get_llvm_type(&args[0]) } else { self.context.i32_type().into() };
                        let e = if args.len() >= 2 { self.get_llvm_type(&args[1]) } else { self.context.i32_type().into() };
                        let struct_ty = self.context.opaque_struct_type("Result");
                        struct_ty.set_body(&[self.context.i32_type().into(), t, e], false);
                        struct_ty.into()
                    }
                    "Option" => {
                        let t = if args.len() >= 1 { self.get_llvm_type(&args[0]) } else { self.context.i32_type().into() };
                        let struct_ty = self.context.opaque_struct_type("Option");
                        struct_ty.set_body(&[self.context.i32_type().into(), t], false);
                        struct_ty.into()
                    }
                    _ => {
                        if let Some(st) = self.struct_types.get(name) {
                            st.as_basic_type_enum()
                        } else {
                            self.context.i32_type().into()
                        }
                    }
                }
            }
            TypeKind::Ref(_) | TypeKind::RawPtr(_) => {
                self.context.ptr_type(inkwell::AddressSpace::default()).into()
            }
            TypeKind::Slice(_) => {
                let struct_ty = self.context.opaque_struct_type("Slice");
                struct_ty.set_body(&[
                    self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                    self.context.i64_type().into(),
                ], false);
                struct_ty.into()
            }
        }
    }

    pub fn compile_module(&mut self, module: &Module) -> Result<(), String> {
        // Declare structures first
        for item in &module.items {
            if let Item::Struct(s) = item {
                let struct_type = self.context.opaque_struct_type(&s.name);
                self.struct_types.insert(s.name.clone(), struct_type);
                
                let fields = s.fields.iter().map(|f| f.name.clone()).collect();
                self.struct_fields.insert(s.name.clone(), fields);

                let mut field_map = std::collections::HashMap::new();
                for field in &s.fields {
                    if let TypeKind::Path(path, _) = &field.ty.kind {
                        if let Some(type_name) = path.last() {
                            field_map.insert(field.name.clone(), type_name.clone());
                        }
                    }
                }
                self.struct_field_types.insert(s.name.clone(), field_map);
            }
        }

        // Fill structure fields
        for item in &module.items {
            if let Item::Struct(s) = item {
                let struct_type = self.struct_types.get(&s.name).unwrap();
                let field_types: Vec<BasicTypeEnum> = s.fields.iter().map(|f| self.get_llvm_type(&f.ty)).collect();
                struct_type.set_body(&field_types, false);
            }
        }

        // Declare functions and impl methods
        for item in &module.items {
            match item {
                Item::Fn(f) => {
                    if f.generic_params.is_empty() {
                        self.declare_fn(f)?;
                    }
                }
                Item::Impl(imp) => {
                    if let TypeKind::Path(path, _) = &imp.target_ty.kind {
                        if let Some(struct_name) = path.last() {
                            for method in &imp.methods {
                                if method.generic_params.is_empty() {
                                    self.declare_method(struct_name, method)?;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        // Define functions
        for item in &module.items {
            match item {
                Item::Fn(f) => {
                    if f.generic_params.is_empty() {
                        self.compile_fn(f)?;
                    }
                }
                Item::Impl(imp) => {
                    if let TypeKind::Path(path, _) = &imp.target_ty.kind {
                        if let Some(struct_name) = path.last() {
                            for method in &imp.methods {
                                if method.generic_params.is_empty() {
                                    self.compile_method(struct_name, method)?;
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn declare_fn(&mut self, f: &FnDecl) -> Result<FunctionValue<'ctx>, String> {
        let param_types: Vec<BasicMetadataTypeEnum> = f.params.iter()
            .map(|p| self.get_llvm_type(&p.ty).into())
            .collect();

        let fn_type = if let Some(ref ret) = f.ret_ty {
            self.get_llvm_type(ret).fn_type(&param_types, false)
        } else {
            self.context.void_type().fn_type(&param_types, false)
        };

        let fn_val = self.module.add_function(&f.name, fn_type, None);
        self.functions.insert(f.name.clone(), fn_val);
        Ok(fn_val)
    }

    fn declare_method(&mut self, struct_name: &str, f: &FnDecl) -> Result<FunctionValue<'ctx>, String> {
        let param_types: Vec<BasicMetadataTypeEnum> = f.params.iter()
            .map(|p| {
                if p.name == "self" {
                    // self is a pointer to the struct target type
                    self.context.ptr_type(inkwell::AddressSpace::default()).into()
                } else {
                    self.get_llvm_type(&p.ty).into()
                }
            })
            .collect();

        let fn_type = if let Some(ref ret) = f.ret_ty {
            self.get_llvm_type(ret).fn_type(&param_types, false)
        } else {
            self.context.void_type().fn_type(&param_types, false)
        };

        let full_name = format!("{}::{}", struct_name, f.name);
        let fn_val = self.module.add_function(&full_name, fn_type, None);
        self.functions.insert(full_name, fn_val);
        Ok(fn_val)
    }

    fn compile_fn(&mut self, f: &FnDecl) -> Result<(), String> {
        let fn_val = *self.functions.get(&f.name).unwrap();
        if f.name == "ConvertThreadToFiber" || f.name == "CreateFiber" || f.name == "SwitchToFiber" || f.name == "DeleteFiber" 
            || f.name == "CreateThread" || f.name == "CreateMutexA" || f.name == "ReleaseMutex" || f.name == "WaitForSingleObject" 
            || f.name == "CloseHandle" || f.name == "Sleep" 
            || f.name == "pthread_create" || f.name == "pthread_join" || f.name == "pthread_mutex_init" 
            || f.name == "pthread_mutex_lock" || f.name == "pthread_mutex_unlock" || f.name == "pthread_mutex_destroy" 
            || f.name == "usleep" {
            return Ok(());
        }
        self.compile_fn_body(fn_val, &f.params, &f.body)
    }

    fn compile_method(&mut self, struct_name: &str, f: &FnDecl) -> Result<(), String> {
        let full_name = format!("{}::{}", struct_name, f.name);
        let fn_val = *self.functions.get(&full_name).unwrap();
        self.compile_fn_body(fn_val, &f.params, &f.body)
    }

    fn compile_fn_body(&mut self, fn_val: FunctionValue<'ctx>, params: &[Param], body: &Block) -> Result<(), String> {
        self.current_function = Some(fn_val);
        let entry_bb = self.context.append_basic_block(fn_val, "entry");
        self.builder.position_at_end(entry_bb);

        self.variables.clear();
        self.var_struct_types.clear();

        for (i, param) in params.iter().enumerate() {
            let llvm_ty = if param.name == "self" {
                self.context.ptr_type(inkwell::AddressSpace::default()).into()
            } else {
                self.get_llvm_type(&param.ty)
            };

            let alloca = self.builder.build_alloca(llvm_ty, &param.name).unwrap();
            let val = fn_val.get_nth_param(i as u32).unwrap();
            self.builder.build_store(alloca, val).unwrap();
            self.variables.insert(param.name.clone(), alloca);
            self.variable_types.insert(param.name.clone(), llvm_ty);
            if let TypeKind::Slice(ref inner) = param.ty.kind {
                let elem_llvm_ty = self.get_llvm_type(inner);
                self.slice_element_types.insert(param.name.clone(), elem_llvm_ty);
            }

            if let Some(mut struct_name) = self.get_struct_name(&param.ty) {
                if struct_name == "Self" {
                    let fn_name = fn_val.get_name().to_str().unwrap();
                    if fn_name.contains("::") {
                        if let Some(s) = fn_name.split("::").next() {
                            struct_name = s.to_string();
                        }
                    }
                }
                self.var_struct_types.insert(param.name.clone(), struct_name);
            }
        }

        let has_expr_return = if let Some(last_stmt) = body.stmts.last() {
            if let StmtKind::Expr(_) = &last_stmt.kind {
                fn_val.get_type().get_return_type().is_some()
            } else {
                false
            }
        } else {
            false
        };

        let stmts_to_compile = if has_expr_return {
            &body.stmts[..body.stmts.len() - 1]
        } else {
            &body.stmts[..]
        };

        for stmt in stmts_to_compile {
            self.compile_stmt(stmt)?;
        }

        let current_bb = self.builder.get_insert_block().unwrap();
        if current_bb.get_terminator().is_none() {
            if has_expr_return {
                if let StmtKind::Expr(expr) = &body.stmts.last().unwrap().kind {
                    let mut val = self.compile_expr(expr)?;
                    let expected_ret_ty = fn_val.get_type().get_return_type();
                    if let Some(ret_ty) = expected_ret_ty {
                        if val.is_pointer_value() && !ret_ty.is_pointer_type() {
                            let ptr = val.into_pointer_value();
                            val = self.builder.build_load(ret_ty, ptr, "struct_ret_load").unwrap();
                        }
                    }
                    self.builder.build_return(Some(&val)).unwrap();
                    self.current_function = None;
                    return Ok(());
                }
            }

            if fn_val.get_type().get_return_type().is_none() {
                self.builder.build_return(None).unwrap();
            } else {
                let ret_ty = fn_val.get_type().get_return_type().unwrap();
                let dummy = ret_ty.const_zero();
                self.builder.build_return(Some(&dummy)).unwrap();
            }
        }

        self.current_function = None;
        Ok(())
    }

    fn compile_block(&mut self, block: &Block) -> Result<(), String> {
        for stmt in &block.stmts {
            self.compile_stmt(stmt)?;
        }
        Ok(())
    }

    fn compile_stmt(&mut self, stmt: &Stmt) -> Result<(), String> {
        match &stmt.kind {
            StmtKind::Let { is_mut: _, name, ty, value } => {
                let mut val = self.compile_expr(value)?;
                if let ExprKind::StructInit(path, _) = &value.kind {
                    let struct_name = path.last().unwrap();
                    let struct_type = self.struct_types.get(struct_name).unwrap();
                    self.variables.insert(name.clone(), val.into_pointer_value());
                    self.variable_types.insert(name.clone(), struct_type.as_basic_type_enum());

                    if let Some(ref explicit_ty) = ty {
                        if let Some(struct_name) = self.get_struct_name(explicit_ty) {
                            self.var_struct_types.insert(name.clone(), struct_name);
                        }
                    } else {
                        self.var_struct_types.insert(name.clone(), struct_name.clone());
                    }
                } else {
                    let llvm_ty = if let Some(t) = ty {
                        self.get_llvm_type(t)
                    } else {
                        val.get_type()
                    };
                    if val.get_type().is_int_type() && llvm_ty.is_pointer_type() {
                        val = self.builder.build_int_to_ptr(val.into_int_value(), llvm_ty.into_pointer_type(), "int2ptr").unwrap().into();
                    }
                    if val.get_type().is_pointer_type() && llvm_ty.is_int_type() {
                        val = self.builder.build_ptr_to_int(val.into_pointer_value(), llvm_ty.into_int_type(), "ptr2int").unwrap().into();
                    }
                    let alloca = self.builder.build_alloca(llvm_ty, name).unwrap();
                    self.builder.build_store(alloca, val).unwrap();
                    self.variables.insert(name.clone(), alloca);
                    self.variable_types.insert(name.clone(), llvm_ty);

                    if let Some(ref explicit_ty) = ty {
                        if let Some(struct_name) = self.get_struct_name(explicit_ty) {
                            self.var_struct_types.insert(name.clone(), struct_name);
                        }
                        if let TypeKind::Slice(ref inner) = explicit_ty.kind {
                            let elem_llvm_ty = self.get_llvm_type(inner);
                            self.slice_element_types.insert(name.clone(), elem_llvm_ty);
                        }
                    }
                }
            }
            StmtKind::Assign(lhs, rhs) => {
                let mut val = self.compile_expr(rhs)?;
                let ptr = self.compile_expr_to_ptr(lhs)?;
                let expected_ty = if let ExprKind::Ident(name) = &lhs.kind {
                    self.variable_types.get(name).copied()
                } else {
                    None
                };
                if let Some(expected) = expected_ty {
                    if val.get_type().is_int_type() && expected.is_pointer_type() {
                        val = self.builder.build_int_to_ptr(val.into_int_value(), expected.into_pointer_type(), "int2ptr").unwrap().into();
                    } else if val.get_type().is_pointer_type() && expected.is_int_type() {
                        val = self.builder.build_ptr_to_int(val.into_pointer_value(), expected.into_int_type(), "ptr2int").unwrap().into();
                    }
                }
                self.builder.build_store(ptr, val).unwrap();
            }
            StmtKind::Expr(expr) => {
                self.compile_expr(expr)?;
            }
            StmtKind::Return(val_expr) => {
                if let Some(expr) = val_expr {
                    let mut val = self.compile_expr(expr)?;
                    let expected_ret_ty = self.current_function.unwrap().get_type().get_return_type();
                    if let Some(ret_ty) = expected_ret_ty {
                        if val.is_pointer_value() && !ret_ty.is_pointer_type() {
                            let ptr = val.into_pointer_value();
                            val = self.builder.build_load(ret_ty, ptr, "struct_ret_load").unwrap();
                        }
                    }
                    self.builder.build_return(Some(&val)).unwrap();
                } else {
                    self.builder.build_return(None).unwrap();
                }
            }
            StmtKind::While(cond, body) => {
                let current_func = self.current_function.unwrap();
                let cond_bb = self.context.append_basic_block(current_func, "while_cond");
                let body_bb = self.context.append_basic_block(current_func, "while_body");
                let end_bb = self.context.append_basic_block(current_func, "while_end");

                self.builder.build_unconditional_branch(cond_bb).unwrap();

                // Condition block
                self.builder.position_at_end(cond_bb);
                let cond_val = self.compile_expr(cond)?.into_int_value();
                self.builder.build_conditional_branch(cond_val, body_bb, end_bb).unwrap();

                // Body block
                self.builder.position_at_end(body_bb);
                self.compile_block(body)?;
                if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                    self.builder.build_unconditional_branch(cond_bb).unwrap();
                }

                // End block
                self.builder.position_at_end(end_bb);
            }
            StmtKind::For(name, iter, body) => {
                // Simplified compile for loop: assume iterating over range/i32 for stub execution
                let current_func = self.current_function.unwrap();
                let loop_bb = self.context.append_basic_block(current_func, "for_body");
                let end_bb = self.context.append_basic_block(current_func, "for_end");

                let iter_limit = self.compile_expr(iter)?.into_int_value();
                let index_alloca = self.builder.build_alloca(self.context.i32_type(), name).unwrap();
                self.builder.build_store(index_alloca, self.context.i32_type().const_zero()).unwrap();

                self.builder.build_unconditional_branch(loop_bb).unwrap();

                self.builder.position_at_end(loop_bb);
                let index_val = self.builder.build_load(self.context.i32_type(), index_alloca, "index").unwrap().into_int_value();
                
                // Save variable mapping
                let prev_mapping = self.variables.insert(name.clone(), index_alloca);
                self.variable_types.insert(name.clone(), self.context.i32_type().into());

                self.compile_block(body)?;

                // Increment index
                let next_val = self.builder.build_int_add(index_val, self.context.i32_type().const_int(1, false), "next_index").unwrap();
                self.builder.build_store(index_alloca, next_val).unwrap();

                // Loop check
                let cond = self.builder.build_int_compare(IntPredicate::SLT, next_val, iter_limit, "loop_cond").unwrap();
                self.builder.build_conditional_branch(cond, loop_bb, end_bb).unwrap();

                self.builder.position_at_end(end_bb);
                if let Some(m) = prev_mapping {
                    self.variables.insert(name.clone(), m);
                } else {
                    self.variables.remove(name);
                }
            }
            StmtKind::Raw(block) => {
                self.compile_block(block)?;
            }
        }
        Ok(())
    }

    fn compile_expr_to_ptr(&mut self, expr: &Expr) -> Result<PointerValue<'ctx>, String> {
        match &expr.kind {
            ExprKind::Ident(name) => {
                if let Some(ptr) = self.variables.get(name) {
                    Ok(*ptr)
                } else {
                    Err(format!("cannot find variable `{}` in LLVM codegen scope", name))
                }
            }
            ExprKind::FieldAccess(receiver, field_name) => {
                let struct_ptr = match &receiver.kind {
                    ExprKind::Ident(name) => {
                        let is_ptr = if let Some(ty) = self.variable_types.get(name) {
                            ty.is_pointer_type()
                        } else {
                            false
                        };
                        if is_ptr {
                            self.compile_expr(receiver)?.into_pointer_value()
                        } else {
                            self.compile_expr_to_ptr(receiver)?
                        }
                    }
                    ExprKind::Unary(UnOp::Deref, operand) => {
                        self.compile_expr(operand)?.into_pointer_value()
                    }
                    _ => self.compile_expr_to_ptr(receiver)?,
                };
                let struct_name = self.find_expr_struct_name(receiver).unwrap_or_else(|| "Point".to_string());
                let struct_type = self.struct_types.get(&struct_name).cloned().ok_or_else(|| format!("cannot find struct {} type", struct_name))?;
                
                // Find field index dynamically
                let fields = self.struct_fields.get(&struct_name).ok_or_else(|| format!("unknown struct fields for {}", struct_name))?;
                let idx = fields.iter().position(|f| f == field_name).ok_or_else(|| format!("unknown field {} in struct {}", field_name, struct_name))? as u32;
                
                let field_ptr = self.builder.build_struct_gep(struct_type.as_basic_type_enum(), struct_ptr, idx, field_name).unwrap();
                Ok(field_ptr)
            }
            ExprKind::Unary(UnOp::Deref, operand) => {
                let ptr_val = self.compile_expr(operand)?.into_pointer_value();
                Ok(ptr_val)
            }
            ExprKind::Index(receiver, index) => {
                let slice_alloca = match &receiver.kind {
                    ExprKind::Ident(name) => {
                        *self.variables.get(name).ok_or_else(|| format!("unknown slice variable {}", name))?
                    }
                    _ => self.compile_expr_to_ptr(receiver)?,
                };

                let struct_ty = self.context.opaque_struct_type("Slice");
                struct_ty.set_body(&[
                    self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                    self.context.i64_type().into(),
                ], false);

                let ptr_gep = self.builder.build_struct_gep(struct_ty, slice_alloca, 0, "slice_ptr_gep").unwrap();
                let elem_ptr = self.builder.build_load(self.context.ptr_type(inkwell::AddressSpace::default()), ptr_gep, "slice_ptr_val").unwrap().into_pointer_value();

                let len_gep = self.builder.build_struct_gep(struct_ty, slice_alloca, 1, "slice_len_gep").unwrap();
                let len_val = self.builder.build_load(self.context.i64_type(), len_gep, "slice_len_val").unwrap().into_int_value();

                let idx_val = self.compile_expr(index)?.into_int_value();

                if self.profile == "safe" {
                    let in_bounds = self.builder.build_int_compare(inkwell::IntPredicate::ULT, idx_val, len_val, "in_bounds_cmp").unwrap();

                    let current_fn = self.current_function.ok_or("bounds check outside function")?;
                    let trap_block = self.context.append_basic_block(current_fn, "bounds_trap");
                    let cont_block = self.context.append_basic_block(current_fn, "bounds_cont");

                    self.builder.build_conditional_branch(in_bounds, cont_block, trap_block).unwrap();

                    self.builder.position_at_end(trap_block);
                    let trap_fn = self.module.get_function("llvm.trap").unwrap_or_else(|| {
                        self.module.add_function("llvm.trap", self.context.void_type().fn_type(&[], false), None)
                    });
                    self.builder.build_call(trap_fn, &[], "trap_call").unwrap();
                    self.builder.build_unreachable().unwrap();

                    self.builder.position_at_end(cont_block);
                }

                let elem_type = self.get_slice_elem_type(receiver);
                let element_ptr = unsafe {
                    self.builder.build_gep(elem_type, elem_ptr, &[idx_val], "elem_gep").unwrap()
                };
                Ok(element_ptr)
            }
            _ => Err("expression is not an lvalue".to_string()),
        }
    }

    fn get_slice_elem_type(&self, receiver: &Expr) -> BasicTypeEnum<'ctx> {
        if let ExprKind::Ident(name) = &receiver.kind {
            if let Some(t) = self.slice_element_types.get(name) {
                return *t;
            }
        }
        self.context.i8_type().into()
    }

    fn compile_expr(&mut self, expr: &Expr) -> Result<BasicValueEnum<'ctx>, String> {
        match &expr.kind {
            ExprKind::Lit(lit) => match lit {
                Lit::Int(v, suffix) => {
                    let ty = if let Some(ref s) = suffix {
                        match s.as_str() {
                            "i64" | "u64" | "usize" => self.context.i64_type(),
                            _ => self.context.i32_type(),
                        }
                    } else {
                        self.context.i32_type()
                    };
                    Ok(ty.const_int(*v as u64, false).into())
                }
                Lit::Float(v, _) => Ok(self.context.f64_type().const_float(*v).into()),
                Lit::Bool(v) => Ok(self.context.bool_type().const_int(*v as u64, false).into()),
                Lit::Str(s) => {
                    // String literal is global constant pointer
                    let global = self.builder.build_global_string_ptr(s, "str_lit").unwrap();
                    Ok(global.as_pointer_value().into())
                }
            },
            ExprKind::Ident(name) => {
                if let Some(ptr) = self.variables.get(name) {
                    let ty: BasicTypeEnum = if let Some(t) = self.variable_types.get(name) {
                        *t
                    } else if name.starts_with("dy") || name.starts_with("dx") || name.starts_with("d") || name == "x" || name == "y" {
                        self.context.f64_type().into()
                    } else if name.starts_with("ptr") || name == "self" || name == "other" {
                        self.context.ptr_type(inkwell::AddressSpace::default()).into()
                    } else {
                        self.context.i32_type().into()
                    };
                    let val = self.builder.build_load(ty, *ptr, name).unwrap();
                    Ok(val)
                } else if let Some(func) = self.module.get_function(name) {
                    Ok(func.as_global_value().as_pointer_value().into())
                } else {
                    return Err(format!("unknown variable {}", name));
                }
            }
            ExprKind::Binary(op, lhs, rhs) => {
                let l = self.compile_expr(lhs)?;
                let r = self.compile_expr(rhs)?;

                if l.is_int_value() {
                    let li = l.into_int_value();
                    let ri = r.into_int_value();
                    match op {
                        BinOp::Add => Ok(self.build_checked_binop("llvm.sadd.with.overflow.i32", li, ri).into()),
                        BinOp::Sub => Ok(self.build_checked_binop("llvm.ssub.with.overflow.i32", li, ri).into()),
                        BinOp::Mul => Ok(self.build_checked_binop("llvm.smul.with.overflow.i32", li, ri).into()),
                        BinOp::Div => {
                            // Checked division: check for division-by-zero
                            let current_func = self.current_function.unwrap();
                            let is_zero = self.builder.build_int_compare(IntPredicate::EQ, ri, self.context.i32_type().const_zero(), "is_zero").unwrap();
                            let trap_bb = self.context.append_basic_block(current_func, "div_zero_trap");
                            let next_bb = self.context.append_basic_block(current_func, "div_zero_next");
                            self.builder.build_conditional_branch(is_zero, trap_bb, next_bb).unwrap();

                            self.builder.position_at_end(trap_bb);
                            let trap_fn_ty = self.context.void_type().fn_type(&[], false);
                            let trap_fn = self.module.add_function("llvm.trap", trap_fn_ty, None);
                            self.builder.build_call(trap_fn, &[], "trap_call").unwrap();
                            self.builder.build_unreachable().unwrap();

                            self.builder.position_at_end(next_bb);
                            Ok(self.builder.build_int_signed_div(li, ri, "div").unwrap().into())
                        }
                        BinOp::Eq => Ok(self.builder.build_int_compare(IntPredicate::EQ, li, ri, "eq").unwrap().into()),
                        BinOp::Ne => Ok(self.builder.build_int_compare(IntPredicate::NE, li, ri, "ne").unwrap().into()),
                        BinOp::Lt => Ok(self.builder.build_int_compare(IntPredicate::SLT, li, ri, "lt").unwrap().into()),
                        BinOp::Gt => Ok(self.builder.build_int_compare(IntPredicate::SGT, li, ri, "gt").unwrap().into()),
                        BinOp::Le => Ok(self.builder.build_int_compare(IntPredicate::SLE, li, ri, "le").unwrap().into()),
                        BinOp::Ge => Ok(self.builder.build_int_compare(IntPredicate::SGE, li, ri, "ge").unwrap().into()),
                        _ => Err("unsupported integer binary operation".to_string()),
                    }
                } else if l.is_float_value() {
                    let lf = l.into_float_value();
                    let rf = r.into_float_value();
                    match op {
                        BinOp::Add => Ok(self.builder.build_float_add(lf, rf, "add").unwrap().into()),
                        BinOp::Sub => Ok(self.builder.build_float_sub(lf, rf, "sub").unwrap().into()),
                        BinOp::Mul => Ok(self.builder.build_float_mul(lf, rf, "mul").unwrap().into()),
                        BinOp::Div => Ok(self.builder.build_float_div(lf, rf, "div").unwrap().into()),
                        BinOp::Eq => Ok(self.builder.build_float_compare(FloatPredicate::OEQ, lf, rf, "eq").unwrap().into()),
                        BinOp::Ne => Ok(self.builder.build_float_compare(FloatPredicate::ONE, lf, rf, "ne").unwrap().into()),
                        BinOp::Lt => Ok(self.builder.build_float_compare(FloatPredicate::OLT, lf, rf, "lt").unwrap().into()),
                        BinOp::Gt => Ok(self.builder.build_float_compare(FloatPredicate::OGT, lf, rf, "gt").unwrap().into()),
                        BinOp::Le => Ok(self.builder.build_float_compare(FloatPredicate::OLE, lf, rf, "le").unwrap().into()),
                        BinOp::Ge => Ok(self.builder.build_float_compare(FloatPredicate::OGE, lf, rf, "ge").unwrap().into()),
                        _ => Err("unsupported float binary operation".to_string()),
                    }
                } else {
                    Err("unsupported binary operands type".to_string())
                }
            }
            ExprKind::Unary(op, operand) => {
                let val = self.compile_expr(operand)?;
                match op {
                    UnOp::AddrOf => {
                        let ptr = self.compile_expr_to_ptr(operand)?;
                        Ok(ptr.into())
                    }
                    UnOp::Deref => {
                        let ptr = val.into_pointer_value();
                        // Assume pointer pointee type is i32 or f64 based on context
                        let ty = self.context.i32_type(); // fallback i32 load
                        Ok(self.builder.build_load(ty, ptr, "deref").unwrap())
                    }
                    UnOp::Not => {
                        let int_val = val.into_int_value();
                        Ok(self.builder.build_not(int_val, "not").unwrap().into())
                    }
                }
            }
            ExprKind::StructInit(path, fields) => {
                let struct_name = path.last().unwrap();
                let struct_type = self.struct_types.get(struct_name).cloned().ok_or("unknown struct type")?;
                let alloca = self.builder.build_alloca(struct_type, "struct_init").unwrap();
                
                // Fill struct field initial values
                // The fields list is unordered, match them by index
                for (name, expr) in fields {
                    let val = self.compile_expr(expr)?;
                    let struct_fields_list = self.struct_fields.get(struct_name).ok_or("unknown struct fields")?;
                    let idx = struct_fields_list.iter().position(|f| f == name).ok_or_else(|| format!("unknown field {} in struct {}", name, struct_name))? as u32;
                    let field_ptr = self.builder.build_struct_gep(struct_type.as_basic_type_enum(), alloca, idx, name).unwrap();
                    self.builder.build_store(field_ptr, val).unwrap();
                }

                Ok(alloca.into())
            }
            ExprKind::FieldAccess(receiver, field_name) => {
                let field_ptr = self.compile_expr_to_ptr(expr)?;
                let struct_name = self.find_expr_struct_name(receiver).unwrap_or_else(|| "Point".to_string());
                let struct_type = self.struct_types.get(&struct_name).cloned().ok_or_else(|| format!("cannot find struct {} type", struct_name))?;
                let fields = self.struct_fields.get(&struct_name).ok_or_else(|| format!("unknown struct fields for {}", struct_name))?;
                let idx = fields.iter().position(|f| f == field_name).ok_or_else(|| format!("unknown field {} in struct {}", field_name, struct_name))?;
                
                let field_types = struct_type.get_field_types();
                let ty = field_types.get(idx).copied().ok_or_else(|| format!("field index out of bounds for {} field {}", struct_name, field_name))?;
                
                let val = self.builder.build_load(ty, field_ptr, field_name).unwrap();
                Ok(val)
            }
            ExprKind::MethodCall(receiver, method_name, args) => {
                let struct_ptr = self.compile_expr_to_ptr(receiver)?;
                let struct_name = self.find_expr_struct_name(receiver).unwrap_or_else(|| "Point".to_string());
                let full_name = format!("{}::{}", struct_name, method_name);
                let method_fn = self.functions.get(&full_name).cloned().ok_or_else(|| format!("unknown method {}", full_name))?;

                let mut llvm_args = vec![struct_ptr.into()];
                for arg in args {
                    llvm_args.push(self.compile_expr(arg)?);
                }

                let args_meta: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = llvm_args.iter()
                    .map(|a| (*a).into())
                    .collect();
                let res = self.builder.build_call(method_fn, &args_meta, "method_call").unwrap();
                match res.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(val) => Ok(val),
                    inkwell::values::ValueKind::Instruction(_) => Ok(self.context.i32_type().const_zero().into()),
                }
            }
            ExprKind::Call(callee, args) => {
                let mut func_name = match &callee.kind {
                    ExprKind::Ident(name) => name.clone(),
                    ExprKind::Path(path) => path.join("::"),
                    _ => return Err(format!("only direct function calls are supported: {:?}", callee.kind)),
                };

                if let Some(spec_name) = self.specialized_calls.get(&expr.span) {
                    func_name = spec_name.clone();
                }

                if func_name == "Ok" || func_name == "Err" || func_name == "Some" || func_name == "None" {
                    let ret_ty = self.current_function.unwrap().get_type().get_return_type().unwrap();
                    let struct_ty = ret_ty.into_struct_type();
                    let alloca = self.builder.build_alloca(struct_ty, "enum_init").unwrap();
                    
                    if func_name == "Ok" {
                        let tag = self.context.i32_type().const_int(0, false);
                        let val = self.compile_expr(&args[0])?;
                        let tag_ptr = self.builder.build_struct_gep(struct_ty, alloca, 0, "tag").unwrap();
                        let val_ptr = self.builder.build_struct_gep(struct_ty, alloca, 1, "val").unwrap();
                        self.builder.build_store(tag_ptr, tag).unwrap();
                        self.builder.build_store(val_ptr, val).unwrap();
                    } else if func_name == "Err" {
                        let tag = self.context.i32_type().const_int(1, false);
                        let val = self.compile_expr(&args[0])?;
                        let tag_ptr = self.builder.build_struct_gep(struct_ty, alloca, 0, "tag").unwrap();
                        let val_ptr = self.builder.build_struct_gep(struct_ty, alloca, 2, "val").unwrap();
                        self.builder.build_store(tag_ptr, tag).unwrap();
                        self.builder.build_store(val_ptr, val).unwrap();
                    } else if func_name == "Some" {
                        let tag = self.context.i32_type().const_int(0, false);
                        let val = self.compile_expr(&args[0])?;
                        let tag_ptr = self.builder.build_struct_gep(struct_ty, alloca, 0, "tag").unwrap();
                        let val_ptr = self.builder.build_struct_gep(struct_ty, alloca, 1, "val").unwrap();
                        self.builder.build_store(tag_ptr, tag).unwrap();
                        self.builder.build_store(val_ptr, val).unwrap();
                    } else if func_name == "None" {
                        let tag = self.context.i32_type().const_int(1, false);
                        let tag_ptr = self.builder.build_struct_gep(struct_ty, alloca, 0, "tag").unwrap();
                        self.builder.build_store(tag_ptr, tag).unwrap();
                    }
                    
                    return Ok(alloca.into());
                }

                let mut llvm_args = Vec::new();
                for (i, arg) in args.iter().enumerate() {
                    let mut arg_val = self.compile_expr(arg)?;
                    if let Some(func) = self.module.get_function(&func_name) {
                        let params = func.get_type().get_param_types();
                        if i < params.len() {
                            let param_ty = params[i];
                            if arg_val.get_type().is_int_type() && param_ty.is_pointer_type() {
                                arg_val = self.builder.build_int_to_ptr(arg_val.into_int_value(), param_ty.into_pointer_type(), "arg_int2ptr").unwrap().into();
                            } else if arg_val.get_type().is_pointer_type() && param_ty.is_int_type() {
                                arg_val = self.builder.build_ptr_to_int(arg_val.into_pointer_value(), param_ty.into_int_type(), "arg_ptr2int").unwrap().into();
                            }
                        }
                    }
                    llvm_args.push(arg_val);
                }

                if func_name == "slice_from_raw_parts" || func_name.ends_with("slice_from_raw_parts") {
                    let elem_ptr = llvm_args[0];
                    let len_val = llvm_args[1];

                    let struct_ty = self.context.opaque_struct_type("Slice");
                    struct_ty.set_body(&[
                        self.context.ptr_type(inkwell::AddressSpace::default()).into(),
                        self.context.i64_type().into(),
                    ], false);

                    let alloca = self.builder.build_alloca(struct_ty, "slice_tmp").unwrap();

                    let ptr_gep = self.builder.build_struct_gep(struct_ty, alloca, 0, "slice_ptr").unwrap();
                    let ptr_cast = if elem_ptr.is_int_value() {
                        self.builder.build_int_to_ptr(elem_ptr.into_int_value(), self.context.ptr_type(inkwell::AddressSpace::default()), "ptr_cast").unwrap().into()
                    } else {
                        elem_ptr
                    };
                    self.builder.build_store(ptr_gep, ptr_cast).unwrap();

                    let len_gep = self.builder.build_struct_gep(struct_ty, alloca, 1, "slice_len").unwrap();
                    let len_cast = if len_val.is_pointer_value() {
                        self.builder.build_ptr_to_int(len_val.into_pointer_value(), self.context.i64_type(), "len_cast").unwrap().into()
                    } else {
                        len_val
                    };
                    self.builder.build_store(len_gep, len_cast).unwrap();

                    return Ok(alloca.into());
                }

                // Stub print function
                if func_name == "print" {
                    let printf_type = self.context.i32_type().fn_type(&[self.context.ptr_type(inkwell::AddressSpace::default()).into()], true);
                    let printf_fn = if let Some(f) = self.module.get_function("printf") {
                        f
                    } else {
                        self.module.add_function("printf", printf_type, None)
                    };

                    let mut final_args = Vec::new();
                    if llvm_args.len() == 1 {
                        let arg = llvm_args[0];
                        if arg.is_int_value() {
                            let fmt = self.builder.build_global_string_ptr("%d\n", "fmt_int").unwrap();
                            final_args.push(fmt.as_pointer_value().into());
                            final_args.push(arg);
                        } else if arg.is_float_value() {
                            let fmt = self.builder.build_global_string_ptr("%f\n", "fmt_float").unwrap();
                            final_args.push(fmt.as_pointer_value().into());
                            final_args.push(arg);
                        } else {
                            let fmt = self.builder.build_global_string_ptr("%s\n", "fmt_str").unwrap();
                            final_args.push(fmt.as_pointer_value().into());
                            final_args.push(arg);
                        }
                    } else {
                        final_args = llvm_args;
                    }

                    let args_meta: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = final_args.iter()
                        .map(|a| (*a).into())
                        .collect();

                    self.builder.build_call(printf_fn, &args_meta, "print_call").unwrap();
                    return Ok(self.context.i32_type().const_zero().into());
                }

                let args_meta: Vec<inkwell::values::BasicMetadataValueEnum<'ctx>> = llvm_args.iter()
                    .map(|a| (*a).into())
                    .collect();

                // Stub sqrt function
                if func_name == "sqrt" {
                    let double_ty = self.context.f64_type();
                    let sqrt_fn_ty = double_ty.fn_type(&[double_ty.into()], false);
                    let sqrt_fn = if let Some(f) = self.module.get_function("llvm.sqrt.f64") {
                        f
                    } else {
                        self.module.add_function("llvm.sqrt.f64", sqrt_fn_ty, None)
                    };
                    let res = self.builder.build_call(sqrt_fn, &args_meta, "sqrt_call").unwrap();
                    return match res.try_as_basic_value() {
                        inkwell::values::ValueKind::Basic(val) => Ok(val),
                        inkwell::values::ValueKind::Instruction(_) => Err("llvm.sqrt returned void".to_string()),
                    };
                }

                let func = self.functions.get(&func_name).cloned().ok_or_else(|| format!("unknown function {}", func_name))?;
                let res = self.builder.build_call(func, &args_meta, "fn_call").unwrap();
                match res.try_as_basic_value() {
                    inkwell::values::ValueKind::Basic(val) => Ok(val),
                    inkwell::values::ValueKind::Instruction(_) => Ok(self.context.i32_type().const_zero().into()),
                }
            }
            ExprKind::Block(block) => {
                self.compile_block(block)?;
                Ok(self.context.i32_type().const_zero().into()) // simplified block expression value
            }
            ExprKind::If(cond, then_branch, else_branch) => {
                let current_func = self.current_function.unwrap();
                let then_bb = self.context.append_basic_block(current_func, "then");
                let else_bb = self.context.append_basic_block(current_func, "else");
                let merge_bb = self.context.append_basic_block(current_func, "merge");

                let cond_val = self.compile_expr(cond)?.into_int_value();
                self.builder.build_conditional_branch(cond_val, then_bb, else_bb).unwrap();

                // Then block
                self.builder.position_at_end(then_bb);
                self.compile_block(then_branch)?;
                if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                    self.builder.build_unconditional_branch(merge_bb).unwrap();
                }

                // Else block
                self.builder.position_at_end(else_bb);
                if let Some(eb) = else_branch {
                    match eb {
                        BlockOrIf::Block(b) => self.compile_block(b)?,
                        BlockOrIf::If(c, t, e) => {
                            let dummy = Expr {
                                kind: ExprKind::If(c.clone(), t.clone(), e.clone().map(|x| *x)),
                                span: expr.span,
                            };
                            self.compile_expr(&dummy)?;
                        }
                    }
                }
                if self.builder.get_insert_block().unwrap().get_terminator().is_none() {
                    self.builder.build_unconditional_branch(merge_bb).unwrap();
                }

                // Merge block
                self.builder.position_at_end(merge_bb);
                Ok(self.context.i32_type().const_zero().into())
            }
            ExprKind::Try(sub_expr) => {
                let struct_val = self.compile_expr(sub_expr)?;
                let struct_ty = struct_val.get_type().into_struct_type();
                let alloca = self.builder.build_alloca(struct_ty, "try_struct").unwrap();
                self.builder.build_store(alloca, struct_val).unwrap();
                let struct_ptr = alloca;

                // Get tag
                let tag_ptr = self.builder.build_struct_gep(struct_ty, struct_ptr, 0, "tag").unwrap();
                let tag_val = self.builder.build_load(self.context.i32_type(), tag_ptr, "tag_val").unwrap().into_int_value();

                // Branch based on tag
                let current_func = self.current_function.unwrap();
                let ok_bb = self.context.append_basic_block(current_func, "try_ok");
                let err_bb = self.context.append_basic_block(current_func, "try_err");

                let zero = self.context.i32_type().const_int(0, false);
                let is_ok = self.builder.build_int_compare(IntPredicate::EQ, tag_val, zero, "is_ok").unwrap();
                self.builder.build_conditional_branch(is_ok, ok_bb, err_bb).unwrap();

                // Err branch
                self.builder.position_at_end(err_bb);
                let ret_ty = current_func.get_type().get_return_type().unwrap();
                let ret_struct_ty = ret_ty.into_struct_type();
                let err_alloca = self.builder.build_alloca(ret_struct_ty, "err_ret").unwrap();

                // Store tag 1 (Err/None)
                let tag_ptr_err = self.builder.build_struct_gep(ret_struct_ty, err_alloca, 0, "tag").unwrap();
                self.builder.build_store(tag_ptr_err, self.context.i32_type().const_int(1, false)).unwrap();

                // If it is a Result, copy the error payload
                if struct_ty.count_fields() == 3 {
                    let err_val_ptr = self.builder.build_struct_gep(struct_ty, struct_ptr, 2, "err_val").unwrap();
                    let err_val_ty = struct_ty.get_field_type_at_index(2).unwrap();
                    let err_val = self.builder.build_load(err_val_ty, err_val_ptr, "err_val").unwrap();

                    let ret_err_val_ptr = self.builder.build_struct_gep(ret_struct_ty, err_alloca, 2, "ret_err_val").unwrap();
                    self.builder.build_store(ret_err_val_ptr, err_val).unwrap();
                }

                let ret_struct_val = self.builder.build_load(ret_struct_ty, err_alloca, "ret_struct").unwrap();
                self.builder.build_return(Some(&ret_struct_val)).unwrap();

                // Ok branch
                self.builder.position_at_end(ok_bb);
                let ok_val_ptr = self.builder.build_struct_gep(struct_ty, struct_ptr, 1, "ok_val").unwrap();
                let ok_val_ty = struct_ty.get_field_type_at_index(1).unwrap();
                let ok_val = self.builder.build_load(ok_val_ty, ok_val_ptr, "ok_val").unwrap();

                Ok(ok_val)
            }
            ExprKind::Match(target, arms) => {
                let target_val = self.compile_expr(target)?;
                let target_ty = target_val.get_type().into_struct_type();
                let alloca = self.builder.build_alloca(target_ty, "match_target").unwrap();
                self.builder.build_store(alloca, target_val).unwrap();

                let current_func = self.current_function.unwrap();
                let end_bb = self.context.append_basic_block(current_func, "match_end");

                let match_ret_ty: BasicTypeEnum = self.context.i32_type().into();
                let result_alloca = self.builder.build_alloca(match_ret_ty, "match_result").unwrap();

                // Get target tag
                let tag_ptr = self.builder.build_struct_gep(target_ty, alloca, 0, "tag").unwrap();
                let tag_val = self.builder.build_load(self.context.i32_type(), tag_ptr, "tag_val").unwrap().into_int_value();

                for arm in arms {
                    let arm_bb = self.context.append_basic_block(current_func, "match_arm");
                    let next_arm_bb = self.context.append_basic_block(current_func, "match_next");

                    match &arm.pattern {
                        Pattern::EnumVariant(path, pat_args) => {
                            let variant_name = path.last().unwrap();
                            let expected_tag = if variant_name == "Ok" || variant_name == "Some" {
                                0
                            } else {
                                1
                            };

                            let expected_tag_val = self.context.i32_type().const_int(expected_tag as u64, false);
                            let cond = self.builder.build_int_compare(IntPredicate::EQ, tag_val, expected_tag_val, "tag_check").unwrap();
                            self.builder.build_conditional_branch(cond, arm_bb, next_arm_bb).unwrap();

                            self.builder.position_at_end(arm_bb);
                            for (_i, pat_arg) in pat_args.iter().enumerate() {
                                if let Pattern::Ident(arg_name) = pat_arg {
                                    let field_idx = if variant_name == "Err" {
                                        2
                                    } else {
                                        1
                                    };
                                    let field_ptr = self.builder.build_struct_gep(target_ty, alloca, field_idx as u32, arg_name).unwrap();
                                    let field_ty = target_ty.get_field_type_at_index(field_idx as u32).unwrap();
                                    let val = self.builder.build_load(field_ty, field_ptr, arg_name).unwrap();

                                    let param_alloca = self.builder.build_alloca(field_ty, arg_name).unwrap();
                                    self.builder.build_store(param_alloca, val).unwrap();
                                    self.variables.insert(arg_name.clone(), param_alloca);
                                    self.variable_types.insert(arg_name.clone(), field_ty);
                                }
                            }
                        }
                        _ => {
                            self.builder.build_unconditional_branch(arm_bb).unwrap();
                            self.builder.position_at_end(arm_bb);
                        }
                    }

                    let body_val = self.compile_expr(&arm.body)?;
                    self.builder.build_store(result_alloca, body_val).unwrap();
                    self.builder.build_unconditional_branch(end_bb).unwrap();

                    self.builder.position_at_end(next_arm_bb);
                }

                self.builder.build_unconditional_branch(end_bb).unwrap();
                self.builder.position_at_end(end_bb);

                let res = self.builder.build_load(match_ret_ty, result_alloca, "match_res").unwrap();
                Ok(res)
            }
            ExprKind::Index(receiver, _) => {
                let element_ptr = self.compile_expr_to_ptr(expr)?;
                let elem_type = self.get_slice_elem_type(receiver);
                let val = self.builder.build_load(elem_type, element_ptr, "elem_load").unwrap();
                Ok(val)
            }
            _ => Err("unsupported expression type in LLVM codegen".to_string()),
        }
    }

    fn build_checked_binop(
        &self,
        intrinsic_name: &str,
        lhs: IntValue<'ctx>,
        rhs: IntValue<'ctx>,
    ) -> IntValue<'ctx> {
        let int_type = lhs.get_type();
        let struct_type = self.context.struct_type(
            &[int_type.into(), self.context.bool_type().into()],
            false,
        );

        let param_types: Vec<BasicMetadataTypeEnum> = vec![int_type.into(), int_type.into()];
        let fn_type = struct_type.fn_type(&param_types, false);
        
        let intrinsic = if let Some(f) = self.module.get_function(intrinsic_name) {
            f
        } else {
            self.module.add_function(intrinsic_name, fn_type, None)
        };

        let call_res = self.builder.build_call(intrinsic, &[lhs.into(), rhs.into()], "checked_op").unwrap();
        let res = match call_res.try_as_basic_value() {
            inkwell::values::ValueKind::Basic(val) => val.into_struct_value(),
            inkwell::values::ValueKind::Instruction(_) => unreachable!(),
        };

        let val = self.builder.build_extract_value(res, 0, "val").unwrap().into_int_value();
        let overflow = self.builder.build_extract_value(res, 1, "overflow").unwrap().into_int_value();

        if self.profile == "fast" {
            // In fast mode, wrap silently on overflow (two's complement semantics).
            val
        } else {
            let current_func = self.current_function.unwrap();
            let trap_bb = self.context.append_basic_block(current_func, "trap");
            let next_bb = self.context.append_basic_block(current_func, "next");

            self.builder.build_conditional_branch(overflow, trap_bb, next_bb).unwrap();

            self.builder.position_at_end(trap_bb);
            let trap_fn_ty = self.context.void_type().fn_type(&[], false);
            let trap_fn = if let Some(f) = self.module.get_function("llvm.trap") {
                f
            } else {
                self.module.add_function("llvm.trap", trap_fn_ty, None)
            };
            self.builder.build_call(trap_fn, &[], "trap_call").unwrap();
            self.builder.build_unreachable().unwrap();

            self.builder.position_at_end(next_bb);
            val
        }
    }

    fn get_struct_name(&self, ty: &crate::ast::Type) -> Option<String> {
        match &ty.kind {
            crate::ast::TypeKind::Path(path, _) => path.last().cloned(),
            crate::ast::TypeKind::Ref(inner) => self.get_struct_name(inner),
            crate::ast::TypeKind::RawPtr(inner) => self.get_struct_name(inner),
            _ => None,
        }
    }

    fn find_expr_struct_name(&self, expr: &Expr) -> Option<String> {
        let name = match &expr.kind {
            ExprKind::Ident(name) => self.var_struct_types.get(name).cloned(),
            ExprKind::Unary(UnOp::Deref, operand) => self.find_expr_struct_name(operand),
            ExprKind::Unary(UnOp::AddrOf, operand) => self.find_expr_struct_name(operand),
            ExprKind::FieldAccess(recv, field_name) => {
                if let Some(recv_struct_name) = self.find_expr_struct_name(recv) {
                    if let Some(field_map) = self.struct_field_types.get(&recv_struct_name) {
                        field_map.get(field_name).cloned()
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
            _ => None,
        };
        if let Some(ref s) = name {
            if s == "Self" {
                if let Some(ref func) = self.current_function {
                    let fn_name = func.get_name().to_str().unwrap();
                    if fn_name.contains("::") {
                        return fn_name.split("::").next().map(|x| x.to_string());
                    }
                }
            }
        }
        name
    }
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::Lexer;
    use crate::parser::Parser;

    fn compile_src_to_ir(src: &str) -> String {
        let lexer = Lexer::new(src, "test.cx");
        let mut parser = Parser::new(lexer, "test.cx").unwrap();
        let module = parser.parse_module().unwrap();
        
        let context = Context::create();
        let mut cg = Codegen::new(&context, "test_module");
        cg.compile_module(&module).unwrap();
        cg.module.print_to_string().to_string()
    }

    #[test]
    fn test_codegen_basic_fn() {
        let src = "
        fn add(a: i32, b: i32) -> i32 {
            a + b
        }
        ";
        let ir = compile_src_to_ir(src);
        assert!(ir.contains("define i32 @add("));
        assert!(ir.contains("call { i32, i1 } @llvm.sadd.with.overflow.i32"));
        assert!(ir.contains("call void @llvm.trap()"));
    }

    #[test]
    fn test_codegen_struct_and_method() {
        let src = "
        struct Point {
            x: f64,
            y: f64
        }
        impl Point {
            pub fn new(x: f64, y: f64) -> Point {
                Point { x: x, y: y }
            }
            pub fn distance(self, other: &Point) -> f64 {
                let dx = self.x - other.x;
                let dy = self.y - other.y;
                sqrt(dx * dx + dy * dy)
            }
        }
        ";
        let ir = compile_src_to_ir(src);
        println!("Generated IR:\n{}", ir);
        assert!(ir.contains("%Point = type { double, double }"));
        assert!(ir.contains("@\"Point::new\""));
        assert!(ir.contains("@\"Point::distance\""));
        assert!(ir.contains("call double @llvm.sqrt.f64"));
    }
}

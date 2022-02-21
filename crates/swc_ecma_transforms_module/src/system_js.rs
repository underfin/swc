use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use swc_atoms::{js_word, JsWord};
use swc_common::{BytePos, FileName, Mark, Span, SyntaxContext, DUMMY_SP};
use swc_ecma_ast::*;
use swc_ecma_utils::{quote_ident, quote_str, var::VarCollector, ExprFactory};
use swc_ecma_visit::{noop_fold_type, Fold, FoldWith, VisitWith};

use crate::{
    path::{ImportResolver, NoopImportResolver},
    util::{local_name_for_src, use_strict},
};
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Config {
    #[serde(default)]
    pub allow_top_level_this: bool,
}

struct SystemJs<R>
where
    R: ImportResolver,
{
    root_mark: Mark,
    resolver: Option<(R, FileName)>,
    config: Config,

    declare_var_idents: Vec<Ident>,
    export_map: HashMap<(JsWord, Option<SyntaxContext>), Vec<JsWord>>,
    export_names: Vec<JsWord>,
    export_values: Vec<Expr>,
    export_identifier: Ident,
    context_identifier: Ident,
    tla: bool,
    enter_async_fn: bool,
    enter_fn: bool,
    root_fn_decl_idents: Vec<Ident>,
    module_item_meta_list: Vec<ModuleItemMeta>,
    import_ident_names: Vec<JsWord>,
}

pub fn system_js(root_mark: Mark, config: Config) -> impl Fold {
    SystemJs {
        root_mark,
        resolver: None::<(NoopImportResolver, _)>,
        config,

        declare_var_idents: vec![],
        export_map: HashMap::new(),
        export_names: vec![],
        export_values: vec![],
        export_identifier: quote_ident!("_export"),
        context_identifier: quote_ident!("_context"),
        tla: false,
        enter_async_fn: false,
        enter_fn: false,
        root_fn_decl_idents: vec![],
        module_item_meta_list: vec![],
        import_ident_names: vec![],
    }
}

pub fn system_js_with_resolver<R>(
    resolver: R,
    base: FileName,
    root_mark: Mark,
    config: Config,
) -> impl Fold
where
    R: ImportResolver,
{
    SystemJs {
        root_mark,
        resolver: Some((resolver, base)),
        config,

        declare_var_idents: vec![],
        export_map: HashMap::new(),
        export_names: vec![],
        export_values: vec![],
        export_identifier: quote_ident!("_export"),
        context_identifier: quote_ident!("_context"),
        tla: false,
        enter_async_fn: false,
        enter_fn: false,
        root_fn_decl_idents: vec![],
        module_item_meta_list: vec![],
        import_ident_names: vec![],
    }
}

struct ModuleItemMeta {
    export_names: Vec<JsWord>,
    export_values: Vec<Expr>,
    has_export_all: bool,
    src: JsWord,
    stmts: Vec<Stmt>,
}

impl<R> SystemJs<R>
where
    R: ImportResolver,
{
    fn export_call(&self, name: JsWord, span: Span, expr: ExprOrSpread) -> CallExpr {
        CallExpr {
            span,
            callee: Callee::Expr(Box::new(Expr::Ident(self.export_identifier.clone()))),
            args: vec![Expr::Lit(Lit::Str(quote_str!(name))).as_arg(), expr],
            type_args: None,
        }
    }

    fn fold_module_name_ident(&mut self, ident: Ident) -> Expr {
        if ident.sym.eq("__moduleName") && ident.span.ctxt().outer().eq(&self.root_mark) {
            return Expr::Member(MemberExpr {
                span: DUMMY_SP,
                obj: Box::new(Expr::Ident(self.context_identifier.clone())),
                prop: MemberProp::Ident(quote_ident!("id")),
            });
        }
        Expr::Ident(ident)
    }

    fn replace_assign_expr(&mut self, assign_expr: AssignExpr) -> Expr {
        match &assign_expr.left {
            PatOrExpr::Expr(pat_or_expr) => match &**pat_or_expr {
                Expr::Ident(ident) => {
                    for (k, v) in self.export_map.iter() {
                        if is_eq_sym((ident.sym.clone(), Some(ident.span.ctxt())), k) {
                            let mut expr = Expr::Assign(assign_expr);
                            for value in v.iter() {
                                expr = Expr::Call(self.export_call(
                                    value.clone(),
                                    DUMMY_SP,
                                    expr.as_arg(),
                                ));
                            }
                            return expr;
                        }
                    }
                    Expr::Assign(assign_expr)
                }
                _ => Expr::Assign(assign_expr),
            },
            PatOrExpr::Pat(pat) => {
                let mut to = vec![];
                pat.visit_with(&mut VarCollector { to: &mut to });

                match &**pat {
                    Pat::Ident(ident) => {
                        for (k, v) in self.export_map.iter() {
                            if is_eq_sym((ident.id.sym.clone(), Some(ident.id.span.ctxt())), k) {
                                let mut expr = Expr::Assign(assign_expr);
                                for value in v.iter() {
                                    expr = Expr::Call(self.export_call(
                                        value.clone(),
                                        DUMMY_SP,
                                        expr.as_arg(),
                                    ));
                                }
                                return expr;
                            }
                        }
                        Expr::Assign(assign_expr)
                    }
                    Pat::Object(..) | Pat::Array(..) => {
                        let mut exprs = vec![Box::new(Expr::Assign(assign_expr))];

                        for (sym, ctxt) in to {
                            for (k, v) in self.export_map.iter() {
                                if is_eq_sym((sym.clone(), Some(ctxt)), k) {
                                    for _ in v.iter() {
                                        exprs.push(Box::new(Expr::Call(
                                            self.export_call(
                                                sym.clone(),
                                                DUMMY_SP,
                                                Expr::Ident(Ident::new(
                                                    sym.clone(),
                                                    Span::new(BytePos(0), BytePos(0), ctxt),
                                                ))
                                                .as_arg(),
                                            ),
                                        )));
                                    }
                                    break;
                                }
                            }
                        }
                        Expr::Seq(SeqExpr {
                            span: DUMMY_SP,
                            exprs,
                        })
                    }
                    _ => Expr::Assign(assign_expr),
                }
            }
        }
    }

    fn replace_update_expr(&mut self, update_expr: UpdateExpr) -> Expr {
        if !update_expr.prefix {
            match &*update_expr.arg {
                Expr::Ident(ident) => {
                    for (k, v) in self.export_map.iter() {
                        if is_eq_sym((ident.sym.clone(), Some(ident.span.ctxt)), k) {
                            let mut expr = Expr::Bin(BinExpr {
                                span: DUMMY_SP,
                                op: BinaryOp::Add,
                                left: Box::new(Expr::Unary(UnaryExpr {
                                    span: DUMMY_SP,
                                    op: UnaryOp::Plus,
                                    arg: Box::new(Expr::Ident(ident.clone())),
                                })),
                                right: Box::new(Expr::Lit(Lit::Num(Number {
                                    span: DUMMY_SP,
                                    value: 1.0,
                                }))),
                            });
                            for value in v.iter() {
                                expr = Expr::Call(self.export_call(
                                    value.clone(),
                                    DUMMY_SP,
                                    expr.as_arg(),
                                ));
                            }
                            return Expr::Seq(SeqExpr {
                                span: DUMMY_SP,
                                exprs: vec![Box::new(expr), Box::new(Expr::Update(update_expr))],
                            });
                        }
                    }
                    Expr::Update(update_expr)
                }
                _ => Expr::Update(update_expr),
            }
        } else {
            Expr::Update(update_expr)
        }
    }

    fn add_export_name(&mut self, key: (JsWord, Option<SyntaxContext>), value: JsWord) {
        let mut find = false;
        for (k, v) in self.export_map.iter_mut() {
            if is_eq_sym(key.clone(), k) {
                v.push(value.clone());
                find = true;
                break;
            }
        }
        if !find {
            self.export_map.insert(key, vec![value]);
        }
    }

    fn add_declare_var_idents(&mut self, ident: &Ident) {
        self.declare_var_idents.push(ident.clone());
    }

    fn build_export_call(
        &mut self,
        export_names: &mut Vec<JsWord>,
        export_values: &mut Vec<Expr>,
    ) -> Vec<Stmt> {
        match export_names.len() {
            0 => vec![],
            1 => vec![self
                .export_call(
                    export_names.remove(0),
                    DUMMY_SP,
                    export_values.remove(0).as_arg(),
                )
                .into_stmt()],
            _ => {
                let mut props = vec![];
                while !export_names.is_empty() {
                    let sym = export_names.remove(0);

                    props.push(PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                        key: match Ident::verify_symbol(&sym).is_ok() {
                            true => PropName::Ident(quote_ident!(sym)),
                            false => PropName::Str(quote_str!(sym)),
                        },
                        value: Box::new(export_values.remove(0)),
                    }))));
                }
                return vec![CallExpr {
                    span: DUMMY_SP,
                    callee: Callee::Expr(Box::new(Expr::Ident(self.export_identifier.clone()))),
                    args: vec![Expr::Object(ObjectLit {
                        span: DUMMY_SP,
                        props,
                    })
                    .as_arg()],
                    type_args: None,
                }
                .into_stmt()];
            }
        }
    }

    fn build_module_item_export_all(&mut self, mut meta: ModuleItemMeta) -> Vec<Stmt> {
        match meta.has_export_all {
            false => meta.stmts.append(
                &mut self.build_export_call(&mut meta.export_names, &mut meta.export_values),
            ),
            true => {
                let export_obj = quote_ident!("exportObj");
                let key_ident = quote_ident!("key");
                let target = quote_ident!(local_name_for_src(&meta.src));
                meta.stmts.push(Stmt::Decl(Decl::Var(VarDecl {
                    span: DUMMY_SP,
                    kind: VarDeclKind::Var,
                    declare: false,
                    decls: vec![VarDeclarator {
                        span: DUMMY_SP,
                        name: Pat::Expr(Box::new(Expr::Ident(export_obj.clone()))),
                        init: Some(Box::new(Expr::Object(ObjectLit {
                            span: DUMMY_SP,
                            props: vec![],
                        }))),
                        definite: false,
                    }],
                })));
                meta.stmts.push(Stmt::ForIn(ForInStmt {
                    span: DUMMY_SP,
                    left: VarDeclOrPat::VarDecl(VarDecl {
                        span: DUMMY_SP,
                        kind: VarDeclKind::Var,
                        declare: false,
                        decls: vec![VarDeclarator {
                            span: DUMMY_SP,
                            name: Pat::Expr(Box::new(Expr::Ident(key_ident.clone()))),
                            init: None,
                            definite: false,
                        }],
                    }),
                    right: Box::new(Expr::Ident(target.clone())),

                    body: Box::new(Stmt::Block(BlockStmt {
                        span: DUMMY_SP,
                        stmts: vec![Stmt::If(IfStmt {
                            span: DUMMY_SP,
                            test: Box::new(Expr::Bin(BinExpr {
                                span: DUMMY_SP,
                                op: op!("&&"),
                                left: Box::new(
                                    key_ident
                                        .clone()
                                        .make_bin(op!("!=="), quote_str!("default")),
                                ),
                                right: Box::new(
                                    key_ident
                                        .clone()
                                        .make_bin(op!("!=="), quote_str!("__esModule")),
                                ),
                            })),
                            cons: Box::new(Stmt::Block(BlockStmt {
                                span: DUMMY_SP,
                                stmts: vec![AssignExpr {
                                    span: DUMMY_SP,
                                    op: op!("="),
                                    left: PatOrExpr::Expr(Box::new(
                                        export_obj.clone().computed_member(key_ident.clone()),
                                    )),
                                    right: Box::new(target.computed_member(key_ident)),
                                }
                                .into_stmt()],
                            })),
                            alt: None,
                        })],
                    })),
                }));

                while !meta.export_names.is_empty() {
                    meta.stmts.push(
                        AssignExpr {
                            span: DUMMY_SP,
                            op: op!("="),
                            left: PatOrExpr::Expr(Box::new(Expr::Member(MemberExpr {
                                span: DUMMY_SP,
                                obj: Box::new(Expr::Ident(export_obj.clone())),
                                prop: MemberProp::Ident(quote_ident!(meta.export_names.remove(0))),
                            }))),
                            right: Box::new(meta.export_values.remove(0)),
                        }
                        .into_stmt(),
                    );
                }
                meta.stmts.push(
                    CallExpr {
                        span: DUMMY_SP,
                        callee: Callee::Expr(Box::new(Expr::Ident(self.export_identifier.clone()))),
                        args: vec![Expr::Ident(export_obj).as_arg()],
                        type_args: None,
                    }
                    .into_stmt(),
                );
            }
        }
        meta.stmts
    }

    fn add_module_item_meta(&mut self, mut m: ModuleItemMeta) {
        match self
            .module_item_meta_list
            .iter()
            .position(|i| m.src.eq(&i.src))
        {
            Some(index) => {
                let mut meta = self.module_item_meta_list.remove(index);
                meta.stmts.append(&mut m.stmts);
                meta.export_names.append(&mut m.export_names);
                meta.export_values.append(&mut m.export_values);
                if m.has_export_all {
                    meta.has_export_all = m.has_export_all;
                }
                self.module_item_meta_list.insert(index, meta);
            }
            None => {
                self.module_item_meta_list.push(m);
            }
        }
    }

    fn hoist_var_decl(&mut self, var_decl: VarDecl) -> Option<Expr> {
        let mut exprs = vec![];
        for var_declartor in var_decl.decls {
            let mut tos = vec![];
            var_declartor.visit_with(&mut VarCollector { to: &mut tos });

            for (sym, ctxt) in tos {
                if var_declartor.init.is_none() {
                    for (k, v) in self.export_map.iter_mut() {
                        if is_eq_sym((sym.clone(), Some(ctxt)), k) {
                            for value in v.iter() {
                                self.export_names.push(value.clone());
                                self.export_values.push(undefined_expr(DUMMY_SP));
                            }
                            break;
                        }
                    }
                }
                self.declare_var_idents
                    .push(Ident::new(sym, Span::new(BytePos(0), BytePos(0), ctxt)));
            }

            if let Some(init) = var_declartor.init {
                exprs.push(Expr::Assign(AssignExpr {
                    span: DUMMY_SP,
                    op: op!("="),
                    left: PatOrExpr::Pat(Box::new(var_declartor.name)),
                    right: init,
                }));
            }
        }
        match exprs.len() {
            0 => None,
            _ => Some(Expr::Seq(SeqExpr {
                span: DUMMY_SP,
                exprs: exprs.into_iter().map(Box::new).collect(),
            })),
        }
    }

    fn hoist_for_var_decl(&mut self, var_decl_or_pat: VarDeclOrPat) -> VarDeclOrPat {
        if let VarDeclOrPat::VarDecl(mut var_decl) = var_decl_or_pat {
            if var_decl.kind == VarDeclKind::Var {
                let var_declartor = var_decl.decls.remove(0);
                let mut tos = vec![];
                var_declartor.visit_with(&mut VarCollector { to: &mut tos });

                for (sym, ctxt) in tos {
                    if var_declartor.init.is_none() {
                        for (k, v) in self.export_map.iter_mut() {
                            if is_eq_sym((sym.clone(), Some(ctxt)), k) {
                                for value in v.iter() {
                                    self.export_names.push(value.clone());
                                    self.export_values.push(undefined_expr(DUMMY_SP));
                                }
                                break;
                            }
                        }
                    }
                    self.declare_var_idents
                        .push(Ident::new(sym, Span::new(BytePos(0), BytePos(0), ctxt)));
                }

                VarDeclOrPat::Pat(var_declartor.name)
            } else {
                VarDeclOrPat::VarDecl(var_decl)
            }
        } else {
            var_decl_or_pat
        }
    }

    fn hoist_variables(&mut self, stmt: Stmt) -> Stmt {
        match stmt {
            Stmt::Decl(decl) => {
                if let Decl::Var(var_decl) = decl {
                    // if var_decl.kind == VarDeclKind::Var {
                    if let Some(expr) = self.hoist_var_decl(var_decl) {
                        expr.into_stmt()
                    } else {
                        Stmt::Empty(EmptyStmt { span: DUMMY_SP })
                    }
                    // } else {
                    //     return Stmt::Decl(Decl::Var(var_decl));
                    // }
                } else {
                    Stmt::Decl(decl)
                }
            }
            Stmt::For(for_stmt) => {
                if let Some(init) = for_stmt.init {
                    if let VarDeclOrExpr::VarDecl(var_decl) = init {
                        if var_decl.kind == VarDeclKind::Var {
                            Stmt::For(ForStmt {
                                init: self
                                    .hoist_var_decl(var_decl)
                                    .map(|expr| VarDeclOrExpr::Expr(Box::new(expr))),
                                ..for_stmt
                            })
                        } else {
                            Stmt::For(ForStmt {
                                init: Some(VarDeclOrExpr::VarDecl(var_decl)),
                                ..for_stmt
                            })
                        }
                    } else {
                        Stmt::For(ForStmt {
                            init: Some(init),
                            ..for_stmt
                        })
                    }
                } else {
                    Stmt::For(for_stmt)
                }
            }
            Stmt::ForIn(for_in_stmt) => Stmt::ForIn(ForInStmt {
                left: self.hoist_for_var_decl(for_in_stmt.left),
                ..for_in_stmt
            }),
            Stmt::ForOf(for_of_stmt) => Stmt::ForOf(ForOfStmt {
                left: self.hoist_for_var_decl(for_of_stmt.left),
                ..for_of_stmt
            }),
            _ => stmt,
        }
    }
}

impl<R> Fold for SystemJs<R>
where
    R: ImportResolver,
{
    noop_fold_type!();

    fn fold_expr(&mut self, expr: Expr) -> Expr {
        match expr {
            Expr::Ident(ident) => self.fold_module_name_ident(ident),
            Expr::Assign(assign) => {
                let assign_expr = AssignExpr {
                    right: match *assign.right {
                        Expr::Ident(ident) => Box::new(self.fold_module_name_ident(ident)),
                        _ => assign.right,
                    },
                    ..assign
                }
                .fold_with(self);
                self.replace_assign_expr(assign_expr)
            }
            Expr::Update(update) => self.replace_update_expr(update),
            Expr::Call(call) => match call.callee {
                Callee::Import(_) => Expr::Call(CallExpr {
                    args: call.args.fold_with(self),
                    callee: Callee::Expr(Box::new(Expr::Member(MemberExpr {
                        span: DUMMY_SP,
                        obj: Box::new(Expr::Ident(self.context_identifier.clone())),
                        prop: MemberProp::Ident(quote_ident!("import")),
                    }))),
                    ..call
                }),
                _ => Expr::Call(CallExpr {
                    args: call.args.fold_with(self),
                    ..call
                }),
            },
            Expr::MetaProp(meta_prop_expr) => match meta_prop_expr.kind {
                MetaPropKind::ImportMeta => Expr::Member(MemberExpr {
                    span: DUMMY_SP,
                    obj: Box::new(Expr::Ident(self.context_identifier.clone())),
                    prop: MemberProp::Ident(quote_ident!("meta")),
                }),
                _ => Expr::MetaProp(meta_prop_expr),
            },
            Expr::Await(await_expr) => {
                if !self.enter_async_fn {
                    self.tla = true;
                }
                Expr::Await(await_expr)
            }
            Expr::This(this_expr) => {
                if !self.config.allow_top_level_this && !self.enter_fn {
                    return undefined_expr(DUMMY_SP);
                }
                Expr::This(this_expr)
            }
            _ => expr.fold_children_with(self),
        }
    }

    fn fold_fn_decl(&mut self, fn_decl: FnDecl) -> FnDecl {
        let is_async = fn_decl.function.is_async;
        if is_async {
            self.enter_async_fn = true;
        }
        self.enter_fn = true;
        let fold_fn_expr = fn_decl.fold_children_with(self);
        if is_async {
            self.enter_async_fn = false;
        }
        self.enter_fn = false;
        fold_fn_expr
    }

    fn fold_prop(&mut self, prop: Prop) -> Prop {
        match prop {
            Prop::Shorthand(shorthand) => Prop::KeyValue(KeyValueProp {
                key: PropName::Ident(shorthand.clone()),
                value: Box::new(self.fold_module_name_ident(shorthand)),
            }),
            Prop::KeyValue(key_value_prop) => Prop::KeyValue(KeyValueProp {
                key: key_value_prop.key,
                value: match *key_value_prop.value {
                    Expr::Ident(ident) => Box::new(self.fold_module_name_ident(ident)),
                    _ => key_value_prop.value,
                },
            }),
            _ => prop,
        }
    }

    fn fold_module(&mut self, module: Module) -> Module {
        let mut before_body_stmts: Vec<Stmt> = vec![];
        let mut execute_stmts = vec![];

        // collect top level fn decl
        for item in &module.body {
            if let ModuleItem::Stmt(Stmt::Decl(Decl::Fn(fn_decl))) = item {
                self.root_fn_decl_idents.push(fn_decl.ident.clone());
            }
        }

        for item in module.body {
            match item {
                ModuleItem::ModuleDecl(decl) => match decl {
                    ModuleDecl::Import(import) => {
                        let src = match &self.resolver {
                            Some((resolver, base)) => resolver
                                .resolve_import(base, &import.src.value)
                                .with_context(|| {
                                    format!("failed to resolve import `{}`", import.src.value)
                                })
                                .unwrap(),
                            None => import.src.value,
                        };

                        let source_alias = local_name_for_src(&src);

                        let mut setter_fn_stmts = vec![];

                        for specifier in import.specifiers {
                            match specifier {
                                ImportSpecifier::Default(specifier) => {
                                    self.import_ident_names.push(specifier.local.sym.clone());
                                    self.add_declare_var_idents(&specifier.local);
                                    setter_fn_stmts.push(
                                        AssignExpr {
                                            span: specifier.span,
                                            op: op!("="),
                                            left: PatOrExpr::Expr(Box::new(Expr::Ident(
                                                specifier.local,
                                            ))),
                                            right: Box::new(Expr::Member(MemberExpr {
                                                span: DUMMY_SP,
                                                obj: Box::new(Expr::Ident(quote_ident!(
                                                    source_alias.clone()
                                                ))),
                                                prop: MemberProp::Ident(quote_ident!("default")),
                                            })),
                                        }
                                        .into_stmt(),
                                    );
                                }
                                ImportSpecifier::Named(specifier) => {
                                    self.add_declare_var_idents(&specifier.local);
                                    setter_fn_stmts.push(
                                        AssignExpr {
                                            span: specifier.span,
                                            op: op!("="),
                                            left: PatOrExpr::Expr(Box::new(Expr::Ident(
                                                specifier.local.clone(),
                                            ))),
                                            right: Box::new(Expr::Member(MemberExpr {
                                                span: DUMMY_SP,
                                                obj: Box::new(Expr::Ident(quote_ident!(
                                                    source_alias.clone()
                                                ))),
                                                prop: match specifier.imported {
                                                    Some(m) => get_module_export_member_prop(&m),
                                                    None => MemberProp::Ident(specifier.local),
                                                },
                                            })),
                                        }
                                        .into_stmt(),
                                    );
                                }
                                ImportSpecifier::Namespace(specifier) => {
                                    self.import_ident_names.push(specifier.local.sym.clone());
                                    self.add_declare_var_idents(&specifier.local);
                                    setter_fn_stmts.push(
                                        AssignExpr {
                                            span: specifier.span,
                                            op: op!("="),
                                            left: PatOrExpr::Expr(Box::new(Expr::Ident(
                                                specifier.local,
                                            ))),
                                            right: Box::new(Expr::Ident(quote_ident!(
                                                source_alias.clone()
                                            ))),
                                        }
                                        .into_stmt(),
                                    );
                                }
                            }
                        }

                        self.add_module_item_meta(ModuleItemMeta {
                            export_names: vec![],
                            export_values: vec![],
                            has_export_all: false,
                            src: src.clone(),
                            stmts: setter_fn_stmts,
                        });
                    }
                    ModuleDecl::ExportNamed(decl) => match decl.src {
                        Some(str) => {
                            let src = match &self.resolver {
                                Some((resolver, base)) => resolver
                                    .resolve_import(base, &str.value)
                                    .with_context(|| {
                                        format!("failed to resolve import `{}`", str.value)
                                    })
                                    .unwrap(),
                                None => str.value,
                            };
                            for specifier in decl.specifiers {
                                let source_alias = local_name_for_src(&src);
                                let mut export_names = vec![];
                                let mut export_values = vec![];

                                match specifier {
                                    ExportSpecifier::Named(specifier) => {
                                        export_names.push(match &specifier.exported {
                                            Some(m) => get_module_export_name(m).0,
                                            None => get_module_export_name(&specifier.orig).0,
                                        });
                                        export_values.push(Expr::Member(MemberExpr {
                                            span: DUMMY_SP,
                                            obj: Box::new(Expr::Ident(quote_ident!(
                                                source_alias.clone()
                                            ))),
                                            prop: get_module_export_member_prop(&specifier.orig),
                                        }));
                                    }
                                    ExportSpecifier::Default(specifier) => {
                                        export_names.push(specifier.exported.sym.clone());
                                        export_values.push(Expr::Member(MemberExpr {
                                            span: DUMMY_SP,
                                            obj: Box::new(Expr::Ident(quote_ident!(
                                                source_alias.clone()
                                            ))),
                                            prop: MemberProp::Ident(quote_ident!("default")),
                                        }));
                                    }
                                    ExportSpecifier::Namespace(specifier) => {
                                        export_names
                                            .push(get_module_export_name(&specifier.name).0);
                                        export_values
                                            .push(Expr::Ident(quote_ident!(source_alias.clone())));
                                    }
                                }

                                self.add_module_item_meta(ModuleItemMeta {
                                    export_names,
                                    export_values,
                                    has_export_all: false,
                                    src: src.clone(),
                                    stmts: vec![],
                                });
                            }
                        }
                        None => {
                            for specifier in decl.specifiers {
                                if let ExportSpecifier::Named(specifier) = specifier {
                                    let (orig_sym, ctxt) = get_module_export_name(&specifier.orig);

                                    if self.root_fn_decl_idents.iter().any(|i| i.sym.eq(&orig_sym))
                                    {
                                        // hoisted function export
                                        self.export_names.push(match &specifier.exported {
                                            Some(m) => get_module_export_name(m).0,
                                            None => orig_sym.clone(),
                                        });
                                        self.export_values
                                            .push(get_module_export_expr(&specifier.orig));
                                    }
                                    if self.import_ident_names.iter().any(|i| orig_sym.eq(i)) {
                                        execute_stmts.push(
                                            self.export_call(
                                                orig_sym.clone(),
                                                DUMMY_SP,
                                                ExprOrSpread {
                                                    expr: Box::new(match &specifier.exported {
                                                        Some(m) => get_module_export_expr(m),
                                                        None => {
                                                            get_module_export_expr(&specifier.orig)
                                                        }
                                                    }),
                                                    spread: None,
                                                },
                                            )
                                            .into_stmt(),
                                        );
                                    }
                                    self.add_export_name(
                                        (orig_sym, ctxt),
                                        match specifier.exported {
                                            Some(m) => get_module_export_name(&m).0,
                                            None => get_module_export_name(&specifier.orig).0,
                                        },
                                    );
                                }
                            }
                        }
                    },
                    ModuleDecl::ExportDecl(decl) => {
                        match decl.decl {
                            Decl::Class(class_decl) => {
                                let ident = class_decl.ident;
                                self.export_names.push(ident.sym.clone());
                                self.export_values.push(undefined_expr(DUMMY_SP));
                                self.add_declare_var_idents(&ident);
                                self.add_export_name(
                                    (ident.sym.clone(), Some(ident.span.ctxt())),
                                    ident.sym.clone(),
                                );
                                execute_stmts.push(
                                    AssignExpr {
                                        span: DUMMY_SP,
                                        op: op!("="),
                                        left: PatOrExpr::Expr(Box::new(Expr::Ident(ident.clone()))),
                                        right: Box::new(Expr::Class(ClassExpr {
                                            ident: Some(ident.clone()),
                                            class: class_decl.class,
                                        })),
                                    }
                                    .into_stmt(),
                                );
                            }
                            Decl::Fn(fn_decl) => {
                                self.export_names.push(fn_decl.ident.sym.clone());
                                self.export_values.push(Expr::Ident(fn_decl.ident.clone()));
                                self.add_export_name(
                                    (fn_decl.ident.sym.clone(), Some(fn_decl.ident.span.ctxt())),
                                    fn_decl.ident.sym.clone(),
                                );
                                before_body_stmts.push(Stmt::Decl(Decl::Fn(fn_decl)));
                            }
                            Decl::Var(var_decl) => {
                                let mut decl = VarDecl {
                                    decls: vec![],
                                    ..var_decl
                                };
                                for var_declartor in var_decl.decls {
                                    let mut to = vec![];
                                    var_declartor.visit_with(&mut VarCollector { to: &mut to });
                                    for (sym, ctxt) in to {
                                        let ident = Ident::new(
                                            sym.clone(),
                                            Span::new(BytePos(0), BytePos(0), ctxt),
                                        );
                                        self.add_export_name((sym, Some(ctxt)), ident.sym.clone());
                                    }
                                    decl.decls.push(var_declartor);
                                }
                                execute_stmts.push(Stmt::Decl(Decl::Var(decl)));
                            }
                            _ => {}
                        };
                    }
                    ModuleDecl::ExportDefaultDecl(decl) => {
                        match decl.decl {
                            DefaultDecl::Class(class_expr) => {
                                if let Some(ident) = &class_expr.ident {
                                    self.export_names.push(js_word!("default"));
                                    self.export_values.push(undefined_expr(DUMMY_SP));
                                    self.add_declare_var_idents(ident);
                                    self.add_export_name(
                                        (ident.sym.clone(), Some(ident.span.ctxt())),
                                        js_word!("default"),
                                    );
                                    execute_stmts.push(
                                        AssignExpr {
                                            span: DUMMY_SP,
                                            op: op!("="),
                                            left: PatOrExpr::Expr(Box::new(Expr::Ident(
                                                ident.clone(),
                                            ))),
                                            right: Box::new(Expr::Class(class_expr)),
                                        }
                                        .into_stmt(),
                                    );
                                } else {
                                    self.export_names.push(js_word!("default"));
                                    self.export_values.push(Expr::Class(class_expr));
                                }
                            }
                            DefaultDecl::Fn(fn_expr) => {
                                if let Some(ident) = &fn_expr.ident {
                                    self.export_names.push(js_word!("default"));
                                    self.export_values.push(Expr::Ident(ident.clone()));
                                    self.add_export_name(
                                        (ident.sym.clone(), Some(ident.span.ctxt())),
                                        js_word!("default"),
                                    );
                                    before_body_stmts.push(Stmt::Decl(Decl::Fn(FnDecl {
                                        ident: ident.clone(),
                                        declare: false,
                                        function: fn_expr.function,
                                    })));
                                } else {
                                    self.export_names.push(js_word!("default"));
                                    self.export_values.push(Expr::Fn(fn_expr));
                                }
                            }
                            _ => {}
                        };
                    }
                    ModuleDecl::ExportDefaultExpr(expr) => {
                        execute_stmts.push(
                            self.export_call(
                                js_word!("default"),
                                expr.span,
                                ExprOrSpread {
                                    expr: expr.expr,
                                    spread: None,
                                },
                            )
                            .into_stmt(),
                        );
                    }
                    ModuleDecl::ExportAll(decl) => {
                        self.add_module_item_meta(ModuleItemMeta {
                            export_names: vec![],
                            export_values: vec![],
                            has_export_all: true,
                            src: decl.src.value,
                            stmts: vec![],
                        });
                    }
                    _ => {}
                },
                ModuleItem::Stmt(stmt) => match stmt {
                    Stmt::Decl(decl) => match decl {
                        Decl::Class(class_decl) => {
                            self.add_declare_var_idents(&class_decl.ident);
                            execute_stmts.push(
                                AssignExpr {
                                    span: DUMMY_SP,
                                    op: op!("="),
                                    left: PatOrExpr::Expr(Box::new(Expr::Ident(
                                        class_decl.ident.clone(),
                                    ))),
                                    right: Box::new(Expr::Class(ClassExpr {
                                        ident: Some(class_decl.ident.clone()),
                                        class: class_decl.class,
                                    })),
                                }
                                .into_stmt(),
                            );
                        }
                        Decl::Fn(fn_decl) => {
                            before_body_stmts.push(Stmt::Decl(Decl::Fn(fn_decl)));
                        }
                        _ => execute_stmts.push(Stmt::Decl(decl)),
                    },
                    _ => execute_stmts.push(stmt),
                },
            }
        }

        // ====================
        //  fold_with function, here export_map is collected finished.
        // ====================

        before_body_stmts = before_body_stmts
            .into_iter()
            .map(|s| s.fold_with(self))
            .collect();

        execute_stmts = execute_stmts
            .into_iter()
            .map(|s| self.hoist_variables(s).fold_with(self))
            .filter(|s| !matches!(s, Stmt::Empty(_)))
            .collect();

        // ====================
        //  generate export call, here export_names is collected finished.
        // ====================
        if !self.export_names.is_empty() {
            let mut export_names = self.export_names.drain(0..).collect();
            let mut export_values = self.export_values.drain(0..).collect();
            before_body_stmts
                .append(&mut self.build_export_call(&mut export_names, &mut export_values));
        }

        let mut setters = ArrayLit {
            span: DUMMY_SP,
            elems: vec![],
        };

        let mut dep_module_names = ArrayLit {
            span: DUMMY_SP,
            elems: vec![],
        };

        while !self.module_item_meta_list.is_empty() {
            let meta = self.module_item_meta_list.remove(0);
            dep_module_names
                .elems
                .push(Some(Lit::Str(quote_str!(meta.src.clone())).as_arg()));
            setters.elems.push(Some(
                FnExpr {
                    ident: None,
                    function: Function {
                        params: vec![Param {
                            span: DUMMY_SP,
                            decorators: Default::default(),
                            pat: quote_ident!(local_name_for_src(&meta.src)).into(),
                        }],
                        decorators: Default::default(),
                        span: DUMMY_SP,
                        body: Some(BlockStmt {
                            span: DUMMY_SP,
                            stmts: self.build_module_item_export_all(meta),
                        }),
                        is_generator: false,
                        is_async: false,
                        type_params: Default::default(),
                        return_type: Default::default(),
                    },
                }
                .as_arg(),
            ));
        }

        let execute = Function {
            params: vec![],
            decorators: Default::default(),
            span: DUMMY_SP,
            body: Some(BlockStmt {
                span: DUMMY_SP,
                stmts: execute_stmts,
            }),
            is_generator: false,
            is_async: self.tla,
            type_params: Default::default(),
            return_type: Default::default(),
        };

        let return_stmt = ReturnStmt {
            span: DUMMY_SP,
            arg: Some(Box::new(Expr::Object(ObjectLit {
                span: DUMMY_SP,
                props: vec![
                    PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                        key: quote_ident!("setters").into(),
                        value: Box::new(Expr::Array(setters)),
                    }))),
                    PropOrSpread::Prop(Box::new(Prop::KeyValue(KeyValueProp {
                        key: quote_ident!("execute").into(),
                        value: Box::new(Expr::Fn(FnExpr {
                            ident: None,
                            function: execute,
                        })),
                    }))),
                ],
            }))),
        };

        let mut function_stmts = vec![use_strict()];

        if !self.declare_var_idents.is_empty() {
            function_stmts.push(Stmt::Decl(Decl::Var(VarDecl {
                span: DUMMY_SP,
                kind: VarDeclKind::Var,
                declare: false,
                decls: self
                    .declare_var_idents
                    .iter()
                    .map(|i| VarDeclarator {
                        span: i.span,
                        name: Pat::Ident(BindingIdent {
                            id: i.clone(),
                            type_ann: None,
                        }),
                        init: None,
                        definite: false,
                    })
                    .collect(),
            })));
        }
        function_stmts.append(&mut before_body_stmts);
        function_stmts.push(return_stmt.into());

        let function = Function {
            params: vec![
                Param {
                    span: DUMMY_SP,
                    decorators: Default::default(),
                    pat: self.export_identifier.clone().into(),
                },
                Param {
                    span: DUMMY_SP,
                    decorators: Default::default(),
                    pat: self.context_identifier.clone().into(),
                },
            ],
            decorators: Default::default(),
            span: DUMMY_SP,
            body: Some(BlockStmt {
                span: DUMMY_SP,
                stmts: function_stmts,
            }),
            is_generator: false,
            is_async: false,
            type_params: Default::default(),
            return_type: Default::default(),
        };

        Module {
            body: vec![CallExpr {
                span: DUMMY_SP,
                callee: quote_ident!("System.register").as_callee(),
                args: vec![
                    dep_module_names.as_arg(),
                    FnExpr {
                        ident: None,
                        function,
                    }
                    .as_arg(),
                ],
                type_args: Default::default(),
            }
            .into_stmt()
            .into()],
            ..module
        }
    }
}

#[inline]
fn get_module_export_name(
    module_export_name: &ModuleExportName,
) -> (JsWord, Option<SyntaxContext>) {
    match &module_export_name {
        ModuleExportName::Ident(ident) => (ident.sym.clone(), Some(ident.span.ctxt())),
        ModuleExportName::Str(str) => (str.value.clone(), None),
    }
}

#[inline]
fn get_module_export_expr(module_export_name: &ModuleExportName) -> Expr {
    match &module_export_name {
        ModuleExportName::Ident(ident) => Expr::Ident(ident.clone()),
        ModuleExportName::Str(str) => Expr::Lit(Lit::Str(quote_str!(str.value.clone()))),
    }
}

#[inline]
fn get_module_export_member_prop(module_export_name: &ModuleExportName) -> MemberProp {
    match &module_export_name {
        ModuleExportName::Ident(ident) => MemberProp::Ident(ident.clone()),
        ModuleExportName::Str(str) => MemberProp::Computed(ComputedPropName {
            span: str.span,
            expr: Box::new(Expr::Lit(Lit::Str(quote_str!(str.value.clone())))),
        }),
    }
}

#[inline]
fn undefined_expr(span: Span) -> Expr {
    Expr::Unary(UnaryExpr {
        span,
        op: op!("void"),
        arg: Expr::Lit(Lit::Num(Number { value: 0.0, span })).into(),
    })
}

#[inline]
fn is_eq_sym(
    actually: (JsWord, Option<SyntaxContext>),
    expected: &(JsWord, Option<SyntaxContext>),
) -> bool {
    match (actually.1, expected.1) {
        (Some(a), Some(e)) => a.eq(&e) && expected.0.eq(&actually.0),
        (Some(_), None) => false,
        (None, Some(_)) => false,
        (None, None) => expected.0.eq(&actually.0),
    }
}

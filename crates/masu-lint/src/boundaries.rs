//! Contracts on the guest memory, parser and storage APIs.
use super::{Hits, Rel, Rule, Tree};
use syn::{spanned::Spanned, visit::{self, Visit}};

pub(super) const MEMORY: &str = "crates/koji/src/lib.rs";
pub(super) const PAGES: &str = "crates/kura/src/pages.rs";
pub(super) const GLSL: &str = "vendor/naga/src/front/glsl/parser/types.rs";
pub(super) const STORAGE: &str = "crates/masu/src/storage.rs";

pub(super) fn check(rule: Rule, tree: &Tree, path: &str) -> Hits {
    let file = Rel::new(path);
    let mut check = Check { rule, file, hits: Vec::new(), method: String::new() };
    if let Some(ast) = tree.syntax(&check.file) {
        check.visit_file(&ast);
    }
    check.hits
}

struct Check {
    rule: Rule,
    file: Rel,
    hits: Hits,
    method: String,
}

impl Check {
    fn report(&mut self, at: impl Spanned, message: &str) {
        self.hits.push((self.file.clone(), crate::syntax::line(at.span()), message.to_owned()));
    }
}

impl<'ast> Visit<'ast> for Check {
    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        if self.rule == Rule::MemoryPointer && item.ident == "Memory" {
            self.method = "Memory".into();
            for part in &item.items {
                if let syn::TraitItem::Fn(f) = part {
                    self.visit_return_type(&f.sig.output);
                }
            }
            self.method.clear();
        }
    }

    fn visit_type_ptr(&mut self, ptr: &'ast syn::TypePtr) {
        if self.rule == Rule::MemoryPointer && self.method == "Memory" {
            self.report(ptr, "raw host pointer returned by Memory");
        }
    }

    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        if !crate::syntax::is_cfg_test(&module.attrs) {
            visit::visit_item_mod(self, module);
        }
    }

    fn visit_item_fn(&mut self, f: &'ast syn::ItemFn) {
        if self.rule == Rule::StoreDeletion && !crate::syntax::is_cfg_test(&f.attrs) {
            let outer = std::mem::take(&mut self.method);
            self.visit_block(&f.block);
            self.method = outer;
        }
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        if self.rule != Rule::MemoryPointer {
            visit::visit_item_impl(self, item);
        }
    }

    fn visit_impl_item_fn(&mut self, f: &'ast syn::ImplItemFn) {
        self.method = f.sig.ident.to_string();
        self.visit_block(&f.block);
    }

    fn visit_expr_cast(&mut self, e: &'ast syn::ExprCast) {
        if self.rule == Rule::PagedReadCast && self.method == "read"
            && matches!(&*e.ty, syn::Type::Path(t) if t.path.is_ident("usize"))
        {
            self.report(e, "unchecked conversion to usize in paged read");
        }
        visit::visit_expr_cast(self, e);
    }

    fn visit_expr_method_call(&mut self, e: &'ast syn::ExprMethodCall) {
        let m = &e.method;
        let array_panic = self.rule == Rule::ShaderLayoutPanic
            && self.method.starts_with("parse_array_specifier")
            && (m == "unwrap" || m == "unwrap_err" || m == "expect_err"
                || (m == "expect" && e.args.len() == 1));
        let bypass_gc = self.rule == Rule::StoreDeletion
            && ((m == "delete_object" && self.method != "delete")
                || (m == "delete" && self.method != "gc"
                    && matches!(&*e.receiver, syn::Expr::Path(p) if p.path.is_ident("self"))));
        if array_panic || bypass_gc {
            self.report(m, &m.to_string());
        }
        visit::visit_expr_method_call(self, e);
    }
}

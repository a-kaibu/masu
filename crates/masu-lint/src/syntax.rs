//! What the rules ask of a parsed file (syn): items outside #[cfg(test)] modules, with lines.
use proc_macro2::Span;
use quote::ToTokens;
use syn::visit::Visit;

pub fn line(span: Span) -> usize {
    span.start().line
}

/// `#[cfg(test)]`, `#[cfg(all(test, ..))]`
pub fn is_cfg_test(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| {
        a.path().is_ident("cfg")
            && matches!(&a.meta, syn::Meta::List(l) if {
                let s: String = l.tokens.to_string().split_whitespace().collect();
                s == "test" || s.strip_prefix("all(").and_then(|r| r.strip_suffix(')')).is_some_and(|r| r.split(',').any(|c| c == "test"))
            })
    })
}

/// `#[repr(C)]`, `#[repr(C, packed)]`...
pub fn is_repr_c(attrs: &[syn::Attribute]) -> bool {
    attrs.iter().any(|a| a.path().is_ident("repr") && matches!(&a.meta, syn::Meta::List(l) if l.tokens.clone().into_iter().next().is_some_and(|t| t.to_string() == "C")))
}

/// the last segment's name of a type path (`mem::Ga` -> "Ga")
pub fn last_name(ty: &syn::Type) -> Option<String> {
    if let syn::Type::Path(p) = ty {
        p.path.segments.last().map(|s| s.ident.to_string())
    } else if let syn::Type::Reference(r) = ty {
        last_name(&r.elem)
    } else if let syn::Type::Paren(p) = ty {
        last_name(&p.elem)
    } else if let syn::Type::Group(g) = ty {
        last_name(&g.elem)
    } else {
        None
    }
}

/// The items of a file outside #[cfg(test)] modules, the ones each rule reads.
#[derive(Default)]
pub struct Items {
    /// free fns (not methods): name, line
    pub fns: Vec<(String, usize)>,
    /// methods: (the impl's self type's name, the trait's name if any, method name, line)
    pub methods: Vec<(Option<String>, Option<String>, String, usize)>,
    /// impls: (self type's name, trait's name, line)
    pub impls: Vec<(Option<String>, Option<String>, usize)>,
    /// consts (items, impl and trait consts): name, line, the value's tokens
    pub consts: Vec<(String, usize, String)>,
    pub traits: Vec<(String, usize)>,
    /// repr(C) structs and unions: name, line
    pub repr_c: Vec<(String, usize)>,
    /// every identifier in code (paths, method names, definitions), with macro bodies' too
    pub idents: Vec<(String, usize)>,
    /// two-segment paths `A::b` (paths, and macro bodies' `A :: b`)
    pub paths2: Vec<(String, String, usize)>,
    /// free fns `(b: &[u8], at: usize)` that build a number with from_le_bytes / from_be_bytes
    /// (a byte reader: kome::le has them): name, line
    pub byte_readers: Vec<(String, usize)>,
}

/// `f` reads a number out of a byte slice at an offset (the shape every crate copied)
fn is_byte_reader(f: &syn::ItemFn) -> bool {
    let ins: Vec<String> = f.sig.inputs.iter().map(|a| match a {
        syn::FnArg::Typed(t) => t.ty.to_token_stream().to_string().replace(' ', ""),
        syn::FnArg::Receiver(_) => "self".into(),
    }).collect();
    let body = f.block.to_token_stream().to_string();
    // (a reader is a line or two: a parser that happens to read a number at its start is not one)
    ins == ["&[u8]", "usize"] && body.len() <= 200 && (body.contains("from_le_bytes") || body.contains("from_be_bytes"))
}

impl Items {
    pub fn of(file: &syn::File) -> Items {
        let mut v = Items::default();
        v.visit_file(file);
        v
    }
}

fn trait_name(t: &Option<(Option<syn::token::Not>, syn::Path, syn::token::For)>) -> Option<String> {
    t.as_ref().and_then(|(_, p, _)| p.segments.last()).map(|s| s.ident.to_string())
}

impl<'a> Visit<'a> for Items {
    fn visit_item_mod(&mut self, m: &'a syn::ItemMod) {
        if !is_cfg_test(&m.attrs) {
            syn::visit::visit_item_mod(self, m);
        }
    }
    fn visit_item_fn(&mut self, f: &'a syn::ItemFn) {
        if is_cfg_test(&f.attrs) {
            return;
        }
        self.fns.push((f.sig.ident.to_string(), line(f.sig.ident.span())));
        if is_byte_reader(f) {
            self.byte_readers.push((f.sig.ident.to_string(), line(f.sig.ident.span())));
        }
        syn::visit::visit_item_fn(self, f);
    }
    fn visit_item_impl(&mut self, i: &'a syn::ItemImpl) {
        if is_cfg_test(&i.attrs) {
            return;
        }
        let ty = last_name(&i.self_ty);
        let tr = trait_name(&i.trait_);
        self.impls.push((ty.clone(), tr.clone(), line(i.impl_token.span)));
        for item in &i.items {
            if let syn::ImplItem::Fn(f) = item {
                self.methods.push((ty.clone(), tr.clone(), f.sig.ident.to_string(), line(f.sig.ident.span())));
            }
        }
        syn::visit::visit_item_impl(self, i);
    }
    fn visit_item_const(&mut self, c: &'a syn::ItemConst) {
        if is_cfg_test(&c.attrs) {
            return;
        }
        self.consts.push((c.ident.to_string(), line(c.ident.span()), c.expr.to_token_stream().to_string()));
        syn::visit::visit_item_const(self, c);
    }
    fn visit_impl_item_const(&mut self, c: &'a syn::ImplItemConst) {
        self.consts.push((c.ident.to_string(), line(c.ident.span()), c.expr.to_token_stream().to_string()));
        syn::visit::visit_impl_item_const(self, c);
    }
    fn visit_item_trait(&mut self, t: &'a syn::ItemTrait) {
        self.traits.push((t.ident.to_string(), line(t.ident.span())));
        syn::visit::visit_item_trait(self, t);
    }
    fn visit_item_struct(&mut self, s: &'a syn::ItemStruct) {
        if is_cfg_test(&s.attrs) {
            return;
        }
        if is_repr_c(&s.attrs) {
            self.repr_c.push((s.ident.to_string(), line(s.ident.span())));
        }
        syn::visit::visit_item_struct(self, s);
    }
    fn visit_item_union(&mut self, u: &'a syn::ItemUnion) {
        if is_repr_c(&u.attrs) {
            self.repr_c.push((u.ident.to_string(), line(u.ident.span())));
        }
        syn::visit::visit_item_union(self, u);
    }
    fn visit_ident(&mut self, i: &'a proc_macro2::Ident) {
        self.idents.push((i.to_string(), line(i.span())));
    }
    fn visit_path(&mut self, p: &'a syn::Path) {
        let segs: Vec<_> = p.segments.iter().collect();
        for w in segs.windows(2) {
            if let [a, b] = w {
                self.paths2.push((a.ident.to_string(), b.ident.to_string(), line(b.ident.span())));
            }
        }
        syn::visit::visit_path(self, p);
    }
    fn visit_macro(&mut self, m: &'a syn::Macro) {
        tokens(m.tokens.clone(), self);
        syn::visit::visit_macro(self, m);
    }
}

/// a macro body's identifiers and `A :: b` pairs (it is not parsed: its tokens are all there is)
fn tokens(ts: proc_macro2::TokenStream, v: &mut Items) {
    use proc_macro2::TokenTree;
    let all: Vec<TokenTree> = ts.into_iter().collect();
    for (k, t) in all.iter().enumerate() {
        match t {
            TokenTree::Ident(i) => {
                v.idents.push((i.to_string(), line(i.span())));
                let next = |d: usize| k.checked_add(d).and_then(|x| all.get(x));
                if let (Some(TokenTree::Punct(p1)), Some(TokenTree::Punct(p2)), Some(TokenTree::Ident(b))) = (next(1), next(2), next(3))
                    && p1.as_char() == ':'
                    && p2.as_char() == ':'
                {
                    v.paths2.push((i.to_string(), b.to_string(), line(b.span())));
                }
            }
            TokenTree::Group(g) => tokens(g.stream(), v),
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
}

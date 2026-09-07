//! Route modules register transport paths without implementing operations.

use anyhow::Result;
use syn::visit::Visit;

use crate::workspace::Workspace;
use crate::{Violation, annotations};

pub const RULE: &str = "route-registration";

pub fn check(ws: &Workspace) -> Result<Vec<Violation>> {
    let mut out = Vec::new();
    for r in ws
        .role_index()?
        .records
        .iter()
        .filter(|r| !r.test && r.file.ends_with("/routes.rs"))
    {
        if annotations::permits(r, RULE) {
            continue;
        }
        if !registration(&r.item) {
            out.push(Violation::at(RULE, &r.id, "routes may only register routers, handlers and middleware; opaque syntax requires an explicit exception", r.location()));
        }
    }
    Ok(out)
}

fn registration(item: &syn::Item) -> bool {
    match item {
        syn::Item::Use(_) | syn::Item::ExternCrate(_) => true,
        syn::Item::Struct(s) => {
            matches!(s.fields, syn::Fields::Unit)
                && s.attrs.iter().any(|a| a.path().is_ident("openapi"))
        }
        syn::Item::Fn(f) if f.sig.asyncness.is_none() => {
            let mut scan = Registration { valid: true };
            scan.visit_block(&f.block);
            scan.valid
        }
        _ => false,
    }
}

struct Registration {
    valid: bool,
}

impl<'ast> Visit<'ast> for Registration {
    fn visit_expr(&mut self, expr: &'ast syn::Expr) {
        if matches!(
            expr,
            syn::Expr::Closure(_)
                | syn::Expr::Async(_)
                | syn::Expr::Await(_)
                | syn::Expr::Loop(_)
                | syn::Expr::ForLoop(_)
                | syn::Expr::While(_)
                | syn::Expr::Match(_)
                | syn::Expr::Assign(_)
                | syn::Expr::Try(_)
                | syn::Expr::Unsafe(_)
        ) {
            self.valid = false;
        }
        syn::visit::visit_expr(self, expr);
    }
    fn visit_expr_call(&mut self, call: &'ast syn::ExprCall) {
        let allowed = if let syn::Expr::Path(p) = &*call.func {
            let parts: Vec<_> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            match parts.last().map(String::as_str) {
                Some("from_ref") => parts.iter().rev().nth(1).is_some_and(|p| p == "FromRef"),
                Some("new" | "default") => parts.iter().rev().nth(1).is_some_and(|p| {
                    p.ends_with("Router") || p.ends_with("Layer") || p == "ServiceBuilder"
                }),
                Some(
                    "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "any" | "on"
                    | "routes" | "router" | "from_fn" | "from_fn_with_state" | "openapi"
                    | "with_openapi" | "into",
                ) => true,
                Some(name) => name.ends_with("_router") || name.ends_with("_routes"),
                None => false,
            }
        } else {
            false
        };
        self.valid &= allowed;
        syn::visit::visit_expr_call(self, call);
    }
    fn visit_expr_method_call(&mut self, call: &'ast syn::ExprMethodCall) {
        self.valid &= matches!(
            call.method.to_string().as_str(),
            "route"
                | "routes"
                | "split_for_parts"
                | "into"
                | "nest"
                | "nest_service"
                | "route_service"
                | "merge"
                | "layer"
                | "route_layer"
                | "with_state"
                | "fallback"
                | "fallback_service"
                | "get"
                | "post"
                | "put"
                | "patch"
                | "delete"
                | "head"
                | "options"
                | "clone"
        );
        syn::visit::visit_expr_method_call(self, call);
    }
    fn visit_macro(&mut self, m: &'ast syn::Macro) {
        use syn::parse::Parser;
        let parser = syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated;
        self.valid &= m.path.segments.last().is_some_and(|p| p.ident == "routes")
            && parser.parse2(m.tokens.clone()).is_ok();
    }
    fn visit_item(&mut self, _: &'ast syn::Item) {
        self.valid = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn routes_register_named_handlers_and_middleware() {
        assert!(registration(&syn::parse_quote!(
            fn routes(state: State) -> Router {
                let installation = axum::extract::FromRef::from_ref(&state);
                provider::router(installation)
            }
        )));
        assert!(registration(&syn::parse_quote!(
            fn routes() -> Router {
                Router::new()
                    .route("/", get(handlers::read))
                    .layer(middleware::from_fn(auth::check))
            }
        )));
        for item in [
            syn::parse_quote!(
                async fn handler() {}
            ),
            syn::parse_quote!(
                fn routes() -> Router {
                    Router::new().route("/", get(|| async { operation().await }))
                }
            ),
            syn::parse_quote!(
                fn routes() -> Router {
                    let token = credentials::resolve();
                    Router::new()
                }
            ),
            syn::parse_quote!(
                fn routes() -> Router {
                    hidden!();
                    Router::new()
                }
            ),
        ] {
            assert!(!registration(&item));
        }
    }
}

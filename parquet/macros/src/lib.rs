use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{
    parse_quote,
    visit_mut::{self, VisitMut},
    Block, Expr, ItemFn,
};

/// Attribute macro to generate both synchronous and asynchronous versions of a
/// function.
/// The asynchronous version is only generated if the "async" feature is enabled.
/// Within the function body, calls wrapped with `generic_async_call(...)` are
/// rewritten to call the asynchronous version (with `_async` suffix and `.await`)
/// in the async function, and left unchanged in the sync function.
///# Example
/// ```ignore
/// #[parquet_macros::generic_async]
/// fn example() {
///     let x = generic_async_call(Foo::bar("baz"));
///     let y = generic_async_call(foo.bar("baz"));
/// }
/// ```
/// Expands to:
/// ```ignore
/// fn example() {
///     let x = Foo::bar("baz");
///     let y = foo.bar("baz");
/// }
///
/// #[cfg(feature = "async")]
/// async fn example_async() {
///     let x = Foo::bar_async("baz").await;
///     let y = foo.bar_async("baz").await;
/// }
/// ```
#[proc_macro_attribute]
pub fn generic_async(_attr: TokenStream, item: TokenStream) -> TokenStream {
    _generic_async(item.into()).into()
}

fn _generic_async(item: proc_macro2::TokenStream) -> proc_macro2::TokenStream {
    let f: ItemFn = syn::parse2(item).unwrap();

    let vis = f.vis.clone();
    let attrs = f.attrs.clone();

    // sync function: same name/signature
    let sync_block = generic_call_rewrite(Mode::Sync, f.block.clone());
    let sync_sig = f.sig.clone();
    let sync_fn = quote! {
        #(#attrs)*
        #vis #sync_sig
        #sync_block
    };

    // async function: suffixed and awaited, only if feature "async"
    let async_block = generic_call_rewrite(Mode::Async, f.block.clone());
    let mut async_sig = f.sig.clone();
    async_sig.ident = format_ident!("{}_async", f.sig.ident.clone());
    async_sig.asyncness = Some(syn::token::Async::default());
    let async_fn = quote! {
        #(#attrs)*
        #[cfg(feature = "async")]
        #vis #async_sig
        #async_block
    };

    quote!(#sync_fn #async_fn)
}

const GENERIC_ASYNC_CALL: &str = "generic_async_call";

enum Mode {
    Sync,
    Async,
}

struct GenericCallRewriter {
    mode: Mode,
}

impl GenericCallRewriter {
    fn rewrite_call(&self, expr: &Expr) -> Expr {
        match expr {
            Expr::Call(call) => match self.mode {
                Mode::Sync => expr.clone(),
                Mode::Async => {
                    let mut call = call.clone();
                    if let Expr::Path(path) = &*call.func {
                        let mut path = path.clone();
                        let name = path.path.segments.last_mut().unwrap();
                        name.ident = format_ident!("{}_async", name.ident);
                        *call.func = Expr::Path(path);
                    }
                    parse_quote!(
                        #call.await
                    )
                }
            },
            Expr::MethodCall(call) => match self.mode {
                Mode::Sync => expr.clone(),
                Mode::Async => {
                    let mut call = call.clone();
                    call.method = format_ident!("{}_async", call.method);
                    parse_quote!(
                        #call.await
                    )
                }
            },
            _ => {
                let err = syn::Error::new_spanned(
                    expr,
                    format!("{GENERIC_ASYNC_CALL} expects a function or method call"),
                );
                Expr::Verbatim(err.to_compile_error())
            }
        }
    }
}

impl VisitMut for GenericCallRewriter {
    fn visit_expr_mut(&mut self, node: &mut Expr) {
        if let Expr::Call(call) = node {
            if let Expr::Path(path) = &*call.func {
                if path.path.is_ident(GENERIC_ASYNC_CALL) {
                    match call.args.len() {
                        1 => {
                            let mut inner = call.args.first().unwrap().clone();
                            visit_mut::visit_expr_mut(self, &mut inner);
                            *node = self.rewrite_call(&inner);
                            return;
                        }
                        _ => {
                            let err = syn::Error::new_spanned(
                                call.args.clone(),
                                format!("{GENERIC_ASYNC_CALL} expects exactly one argument"),
                            );
                            *node = Expr::Verbatim(err.to_compile_error());
                            return;
                        }
                    }
                }
            }
        }

        visit_mut::visit_expr_mut(self, node);
    }
}

fn generic_call_rewrite(mode: Mode, input: Box<Block>) -> Box<Block> {
    let mut input = input.clone();
    GenericCallRewriter { mode }.visit_block_mut(&mut input);
    input
}

#[cfg(test)]
mod test {
    use proc_macro2::TokenStream;
    use quote::quote;

    use crate::_generic_async;

    #[test]
    fn test_expansion_works() {
        let input = quote!(
            /// Example function
            fn example() {
                let x = generic_async_call(Foo::bar("baz"));
                let y = generic_async_call(foo.bar("baz"));
            }
        );

        let expected = quote!(
            /// Example function
            fn example() {
                let x = Foo::bar("baz");
                let y = foo.bar("baz");
            }

            /// Example function
            #[cfg(feature = "async")]
            async fn example_async() {
                let x = Foo::bar_async("baz").await;
                let y = foo.bar_async("baz").await;
            }
        );

        let tree: TokenStream = syn::parse2(input).unwrap();

        let result = _generic_async(tree);

        assert_eq!(result.to_string(), expected.to_string());
    }
}

//! Proc macros for `ledger_me_sdk`.
//!
//! [`host_fn`] is for the *host* side (a native function exposed to a
//! pallet's WASM guest). [`call`] and [`import`] are for the *guest* side
//! (the `#![no_std]` pallet crate itself) — see `ledger_me_sdk_guest`,
//! which re-exports both.
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ForeignItem, ItemFn, ItemForeignMod, ReturnType, Type, parse_macro_input};

/// Token-stream rendering of a type, whitespace stripped, so `Vec < u8 >`
/// and `&  [u8]` both compare equal to `"Vec<u8>"`/`"&[u8]"` regardless of
/// how the type was spaced in source.
fn type_str(ty: &Type) -> String {
    quote!(#ty).to_string().split_whitespace().collect()
}

/// Turns a plain `fn(&[u8]) -> Vec<u8>` into a WASM host function that can
/// be passed directly to `Linker::func_wrap`.
///
/// The annotated function keeps its own name and signature untouched for
/// callers (tests, other Rust code); the macro renames the original body
/// into a private helper and generates a public function with the same
/// name that does the `Caller`/memory/`alloc` marshaling via
/// `ledger_me_sdk::call_bytes_host_fn`.
///
/// Fuel: calling a host function costs the guest **1 fuel, whatever it
/// does** (plus a fixed ~217 for the guest's `alloc` that holds the
/// result), since fuel only counts WASM instructions. Offering a function
/// as a host function (fast, not limited by fuel) or leaving it to the
/// pallets in WASM (slower, metered) is the chain maintainers' decision;
/// see `PalletBuilder::with_host_functions`.
#[proc_macro_attribute]
pub fn host_fn(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input_fn = parse_macro_input!(item as ItemFn);

    if input_fn.sig.inputs.len() != 1 {
        return syn::Error::new_spanned(
            &input_fn.sig,
            "#[host_fn] expects a function with a single `&[u8]` parameter, e.g. `fn f(data: &[u8]) -> Vec<u8>`",
        )
        .to_compile_error()
        .into();
    }

    let vis = &input_fn.vis;
    let fn_name = &input_fn.sig.ident;
    let inner_name = format_ident!("__{}_host_fn_impl", fn_name);
    let inputs = &input_fn.sig.inputs;
    let output = &input_fn.sig.output;
    let block = &input_fn.block;

    let expanded = quote! {
        fn #inner_name(#inputs) #output #block

        #vis fn #fn_name(
            caller: ::ledger_me_sdk::wasmtime::Caller<'_, ::ledger_me_sdk::PalletContext>,
            ptr: u32,
            len: u32,
        ) -> ::ledger_me_sdk::wasmtime::Result<i64> {
            ::ledger_me_sdk::call_bytes_host_fn(caller, ptr, len, #inner_name)
        }
    };

    expanded.into()
}

/// Turns a typed `fn(T) -> R` into a pallet's WASM-exported call function.
///
/// The generated export reads its `(ptr, len)` input out of the guest's own
/// memory, decodes it, calls the original function body (renamed to a
/// private helper, same rename trick as [`host_fn`]), encodes the result
/// and hands it back packed as `(ptr, len)` via `ledger_me_sdk_guest`.
///
/// `T`/`R` of exactly `Vec<u8>` (or `&[u8]` for the input) are passed
/// through as raw bytes, matching `Call::data`/`CallResponse::data`
/// unchanged — this is what keeps a plain byte-in/byte-out pallet wire
/// compatible with a host that never heard of Borsh. Any other type is
/// (de)serialized with `borsh`, so it must implement `BorshSerialize`/
/// `BorshDeserialize`.
///
/// # Example
///
/// ```ignore
/// #[call]
/// fn hash(input: Vec<u8>) -> Vec<u8> {
///     sha256(&input)
/// }
///
/// #[derive(borsh::BorshSerialize, borsh::BorshDeserialize)]
/// struct TransferArgs { to: String, amount: u64 }
///
/// #[call]
/// fn transfer(args: TransferArgs) -> Result<(), String> {
///     // ...
/// #   Ok(())
/// }
/// ```
#[proc_macro_attribute]
pub fn call(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let mut input_fn = parse_macro_input!(item as ItemFn);

    if input_fn.sig.inputs.len() != 1 {
        return syn::Error::new_spanned(
            &input_fn.sig,
            "#[call] expects a function with exactly one parameter, e.g. `fn f(input: Vec<u8>) -> Vec<u8>`",
        )
        .to_compile_error()
        .into();
    }

    let arg = match &input_fn.sig.inputs[0] {
        FnArg::Typed(pat_type) => pat_type.clone(),
        FnArg::Receiver(_) => {
            return syn::Error::new_spanned(&input_fn.sig, "#[call] cannot be used on a method")
                .to_compile_error()
                .into();
        }
    };
    let pat = &arg.pat;
    let ty = &arg.ty;

    let output_ty: Type = match &input_fn.sig.output {
        ReturnType::Default => syn::parse_quote!(()),
        ReturnType::Type(_, ty) => (**ty).clone(),
    };

    let vis = input_fn.vis.clone();
    let fn_name = input_fn.sig.ident.clone();
    let inner_name = format_ident!("__{}_call_impl", fn_name);
    input_fn.sig.ident = inner_name.clone();

    let decode_input = match type_str(ty).as_str() {
        "Vec<u8>" => quote! { let #pat: #ty = input_bytes.to_vec(); },
        "&[u8]" => quote! { let #pat: #ty = input_bytes; },
        _ => quote! {
            let #pat: #ty = ::ledger_me_sdk_guest::borsh::from_slice(input_bytes)
                .expect("#[call]: failed to decode input");
        },
    };

    let encode_output = if type_str(&output_ty) == "Vec<u8>" {
        quote! { let bytes: ::ledger_me_sdk_guest::alloc::vec::Vec<u8> = output; }
    } else {
        quote! {
            let bytes: ::ledger_me_sdk_guest::alloc::vec::Vec<u8> =
                ::ledger_me_sdk_guest::borsh::to_vec(&output)
                    .expect("#[call]: failed to encode output");
        }
    };

    let expanded = quote! {
        #input_fn

        #[unsafe(no_mangle)]
        #vis extern "C" fn #fn_name(ptr: u32, len: u32) -> i64 {
            let input_bytes = unsafe { ::ledger_me_sdk_guest::read_input(ptr, len) };
            #decode_input
            let output: #output_ty = #inner_name(#pat);
            #encode_output
            ::ledger_me_sdk_guest::write_output(bytes)
        }
    };

    expanded.into()
}

/// Turns an `extern "C" { fn foo(T) -> R; }` block of raw host-import
/// declarations into safe, typed wrapper functions.
///
/// `#[import(module = "...")]` marks which WASM import module the
/// functions belong to (matching whatever `with_host_functions` registered
/// them under on the host side). Each `fn name(...)` inside is renamed to a
/// hidden raw import (`#[link_name]`-preserved, so the actual WASM import
/// name doesn't change) and paired with a safe wrapper of the original
/// name that does the encode/call/decode dance.
///
/// Same raw-bytes-vs-`borsh` rule as [`call`]: `Vec<u8>`/`&[u8]` pass
/// through untouched, anything else goes through `borsh`.
///
/// # Example
///
/// ```ignore
/// #[import(module = "crypto")]
/// extern "C" {
///     fn sha256(data: &[u8]) -> Vec<u8>;
/// }
/// ```
#[proc_macro_attribute]
pub fn import(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name_value = parse_macro_input!(attr as syn::MetaNameValue);

    if !name_value.path.is_ident("module") {
        return syn::Error::new_spanned(&name_value.path, "expected `module = \"...\"`")
            .to_compile_error()
            .into();
    }

    let module = match &name_value.value {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(s),
            ..
        }) => s.value(),
        _ => {
            return syn::Error::new_spanned(&name_value.value, "expected a string literal")
                .to_compile_error()
                .into();
        }
    };

    let foreign_mod = parse_macro_input!(item as ItemForeignMod);

    let mut raw_externs = Vec::new();
    let mut wrappers = Vec::new();

    for fi in &foreign_mod.items {
        let f = match fi {
            ForeignItem::Fn(f) => f,
            other => {
                raw_externs.push(quote! { #other });
                continue;
            }
        };

        if f.sig.inputs.len() > 1 {
            return syn::Error::new_spanned(
                &f.sig,
                "#[import] expects at most one parameter, e.g. `fn f(data: &[u8]) -> Vec<u8>`",
            )
            .to_compile_error()
            .into();
        }

        let name = &f.sig.ident;
        let name_str = name.to_string();
        let raw_name = format_ident!("__{}_raw", name);
        let vis = &f.vis;

        let output_ty: Type = match &f.sig.output {
            ReturnType::Default => syn::parse_quote!(()),
            ReturnType::Type(_, ty) => (**ty).clone(),
        };

        let (params, prelude, ptr_expr, len_expr) = match f.sig.inputs.first() {
            None => (quote! {}, quote! {}, quote! { 0 }, quote! { 0 }),
            Some(FnArg::Typed(arg)) => {
                let pat = &arg.pat;
                let ty = &arg.ty;

                let (prelude, ptr_expr, len_expr) = match type_str(ty).as_str() {
                    "Vec<u8>" | "&[u8]" => (
                        quote! {},
                        quote! { #pat.as_ptr() as u32 },
                        quote! { #pat.len() as u32 },
                    ),
                    _ => (
                        quote! {
                            let __bytes = ::ledger_me_sdk_guest::borsh::to_vec(&#pat)
                                .expect("#[import]: failed to encode argument");
                        },
                        quote! { __bytes.as_ptr() as u32 },
                        quote! { __bytes.len() as u32 },
                    ),
                };

                (quote! { #pat: #ty }, prelude, ptr_expr, len_expr)
            }
            Some(FnArg::Receiver(_)) => {
                return syn::Error::new_spanned(&f.sig, "#[import] cannot be used on a method")
                    .to_compile_error()
                    .into();
            }
        };

        let decode_output = if type_str(&output_ty) == "Vec<u8>" {
            quote! { result_bytes.to_vec() }
        } else {
            quote! {
                ::ledger_me_sdk_guest::borsh::from_slice(result_bytes)
                    .expect("#[import]: failed to decode result")
            }
        };

        wrappers.push(quote! {
            #vis fn #name(#params) -> #output_ty {
                #prelude
                let __packed = unsafe { #raw_name(#ptr_expr, #len_expr) };
                let (__result_ptr, __result_len) = ::ledger_me_sdk_guest::unpack_ptr_len(__packed);
                let result_bytes = unsafe { ::ledger_me_sdk_guest::read_input(__result_ptr, __result_len) };
                #decode_output
            }
        });

        raw_externs.push(quote! {
            #[link_name = #name_str]
            fn #raw_name(ptr: u32, len: u32) -> i64;
        });
    }

    let expanded = quote! {
        #[link(wasm_import_module = #module)]
        unsafe extern "C" {
            #(#raw_externs)*
        }

        #(#wrappers)*
    };

    expanded.into()
}

use quote::quote;
use syn::{FnArg, GenericArgument, PatType, PathArguments, ReturnType, Type, TypePath};

/// Extracted components of a function signature used for code generation.
///
/// Fields:
/// - Argument patterns (e.g. variable bindings)
/// - Tuple type combining all argument types
/// - Tuple pattern destructuring all arguments
/// - The inner return type (unwrapped from `Result` if applicable)
/// - Whether the return type is a `Result`
pub(crate) type FnSignatureParts = (
	Vec<Box<syn::Pat>>,
	proc_macro2::TokenStream,
	proc_macro2::TokenStream,
	proc_macro2::TokenStream,
	bool,
);

/// Extract argument patterns, types, and return type info from a function signature.
///
/// The `Result` detection is shallow: it only matches the last path segment named
/// `Result` (e.g. `Result<T, E>`, `anyhow::Result<T>`). Aliased or deeply nested
/// Result types are treated as non-Result returns.
pub(crate) fn extract_fn_signature(sig: &syn::Signature) -> syn::Result<FnSignatureParts> {
	let mut arg_patterns = Vec::new();
	let mut arg_types: Vec<&Box<Type>> = Vec::new();

	for arg in &sig.inputs {
		match arg {
			FnArg::Typed(PatType {
				pat,
				ty,
				..
			}) => {
				arg_patterns.push(pat.clone());
				arg_types.push(ty);
			}
			FnArg::Receiver(r) => {
				return Err(syn::Error::new_spanned(
					r,
					"`self` is not supported in #[surrealism] functions",
				));
			}
		}
	}

	let (tuple_type, tuple_pattern) = if arg_types.is_empty() {
		(quote! { () }, quote! { () })
	} else {
		(quote! { ( #(#arg_types),*, ) }, quote! { ( #(#arg_patterns),*, ) })
	};

	let (result_type, is_result) = match &sig.output {
		ReturnType::Default => (quote! { () }, false),
		ReturnType::Type(_, ty) => {
			if let Type::Path(TypePath {
				path,
				..
			}) = &**ty
			{
				if let Some(last_segment) = path.segments.last() {
					if last_segment.ident == "Result" {
						if let PathArguments::AngleBracketed(args) = &last_segment.arguments {
							if let Some(GenericArgument::Type(inner_type)) = args.args.first() {
								(quote! { #inner_type }, true)
							} else {
								(quote! { #ty }, false)
							}
						} else {
							(quote! { #ty }, false)
						}
					} else {
						(quote! { #ty }, false)
					}
				} else {
					(quote! { #ty }, false)
				}
			} else {
				(quote! { #ty }, false)
			}
		}
	};

	Ok((arg_patterns, tuple_type, tuple_pattern, result_type, is_result))
}

use quote::{format_ident, quote};

/// Generate the sentinel const for compile-time duplicate detection.
pub(crate) fn generate_sentinel(export_name: Option<&str>) -> proc_macro2::TokenStream {
	let sentinel_ident = format_ident!("{}", sentinel_const_name(export_name));
	quote! {
		#[doc(hidden)]
		#[allow(dead_code, non_upper_case_globals)]
		const #sentinel_ident: () = ();
	}
}

/// Generate the registration body (invoke/args/returns fns + inventory submit).
/// For init functions, generates the init wrapper + submit.
#[allow(clippy::too_many_arguments)]
pub(crate) fn generate_registration_body(
	fn_name: &syn::Ident,
	arg_patterns: &[Box<syn::Pat>],
	tuple_type: &proc_macro2::TokenStream,
	tuple_pattern: &proc_macro2::TokenStream,
	result_type: &proc_macro2::TokenStream,
	is_result: bool,
	is_init: bool,
	export_name: Option<&str>,
	writeable: bool,
) -> proc_macro2::TokenStream {
	if is_init {
		let init_call = if is_result {
			quote! { #fn_name().map_err(|e| e.to_string()) }
		} else {
			quote! { #fn_name(); Ok(()) }
		};

		let init_ident = format_ident!("__sr_init_{}", fn_name);

		quote! {
			fn #init_ident() -> Result<(), String> {
				#init_call
			}

			surrealism::inventory::submit!(surrealism::SurrealismInit(#init_ident));
		}
	} else {
		let invoke_ident = format_ident!("__sr_invoke_{}", fn_name);
		let args_ident = format_ident!("__sr_args_{}", fn_name);
		let returns_ident = format_ident!("__sr_returns_{}", fn_name);

		let function_call = if is_result {
			quote! { #fn_name(#(#arg_patterns),*).map_err(|e| e.to_string()) }
		} else {
			quote! { Ok(#fn_name(#(#arg_patterns),*)) }
		};

		let name_expr = match export_name {
			None => quote! { None },
			Some(s) => quote! { Some(#s) },
		};

		quote! {
			fn #invoke_ident(raw_args: &[u8]) -> Result<Vec<u8>, String> {
				use surrealism::types::args::Args;
				use surrealdb_types::SurrealValue;

				let values = surrealdb_types::decode_value_list(raw_args)
					.map_err(|e| e.to_string())?;
				let #tuple_pattern: #tuple_type =
					<#tuple_type as Args>::from_values(values)
						.map_err(|e| e.to_string())?;

				let result: Result<#result_type, String> = #function_call;
				let val = result?;
				let public_val = val.into_value();
				surrealdb_types::encode(&public_val).map_err(|e| e.to_string())
			}

			fn #args_ident() -> Result<Vec<u8>, String> {
				use surrealism::types::args::Args;
				let kinds = <#tuple_type as Args>::kinds();
				surrealdb_types::encode_kind_list(&kinds).map_err(|e| e.to_string())
			}

			fn #returns_ident() -> Result<Vec<u8>, String> {
				use surrealdb_types::SurrealValue;
				let kind = <#result_type as SurrealValue>::kind_of();
				surrealdb_types::encode_kind(&kind).map_err(|e| e.to_string())
			}

			surrealism::inventory::submit!(surrealism::SurrealismEntry {
				name: #name_expr,
				invoke: #invoke_ident,
				args: #args_ident,
				returns: #returns_ident,
				writeable: #writeable,
			});
		}
	}
}

/// Build a unique sentinel const name for compile-time duplicate detection.
///
/// - Default export (`None`) → `__sr_export_default`
/// - Named export (`Some(name)`) → `__sr_export__` + encoded name
///
/// The name part is encoded so `_` and `::` in export names produce
/// valid, collision-free Rust identifiers:
///   - `_`  → `_u`  (u for underscore)
///   - `::` → `_s`  (s for scope separator)
///
/// `_` is escaped first so that a literal `_s` in a name becomes `_us`,
/// which cannot collide with `::` (which becomes `_s`).
fn sentinel_const_name(export_name: Option<&str>) -> String {
	match export_name {
		None => "__sr_export_default".to_string(),
		Some(name) => {
			let encoded = name.replace('_', "_u").replace("::", "_s");
			format!("__sr_export__{encoded}")
		}
	}
}

use proc_macro::TokenStream;
use quote::quote;
use syn::{Item, ItemFn, ItemMod};

use crate::attr::parse_surrealism_attr;
use crate::extract::extract_fn_signature;
use crate::generate::{generate_registration_body, generate_sentinel};

pub(crate) fn handle_function(
	is_default: bool,
	export_name_override: Option<String>,
	is_init: bool,
	input_fn: ItemFn,
) -> TokenStream {
	let fn_name = &input_fn.sig.ident;
	let fn_vis = &input_fn.vis;
	let fn_sig = &input_fn.sig;
	let fn_block = &input_fn.block;

	let (arg_patterns, tuple_type, tuple_pattern, result_type, is_result) =
		match extract_fn_signature(fn_sig) {
			Ok(v) => v,
			Err(e) => return e.to_compile_error().into(),
		};

	let export_name: Option<String> = if is_default {
		None
	} else {
		Some(export_name_override.unwrap_or_else(|| fn_name.to_string()))
	};

	if export_name.as_deref() == Some("default") {
		panic!(
			"`default` is reserved for the default export; use #[surrealism(default)] on the \
			 function that should be the default export instead of naming it \"default\""
		);
	}

	let sentinel = generate_sentinel(export_name.as_deref());
	let registration = generate_registration_body(
		fn_name,
		&arg_patterns,
		&tuple_type,
		&tuple_pattern,
		&result_type,
		is_result,
		is_init,
		export_name.as_deref(),
	);

	let expanded = quote! {
		#fn_vis #fn_sig #fn_block

		#sentinel
		#registration
	};

	TokenStream::from(expanded)
}

pub(crate) fn handle_module(
	is_default: bool,
	export_name_override: Option<String>,
	is_init: bool,
	mut item_mod: ItemMod,
) -> TokenStream {
	if is_default {
		panic!("#[surrealism(default)] cannot be used on a module");
	}
	if is_init {
		panic!("#[surrealism(init)] cannot be used on a module");
	}

	let prefix = export_name_override.unwrap_or_else(|| item_mod.ident.to_string());

	let (brace, items) = item_mod
		.content
		.take()
		.expect("#[surrealism] on mod requires an inline module body (mod foo { ... })");

	let (new_items, sentinels) = process_mod_items(&prefix, items);

	item_mod.content = Some((brace, new_items));

	let expanded = quote! {
		#(#sentinels)*
		#item_mod
	};

	TokenStream::from(expanded)
}

/// Recursively walk items inside a `#[surrealism]` mod, processing annotated
/// functions and nested mods.
fn process_mod_items(prefix: &str, items: Vec<Item>) -> (Vec<Item>, Vec<proc_macro2::TokenStream>) {
	let mut new_items: Vec<Item> = Vec::new();
	let mut sentinels: Vec<proc_macro2::TokenStream> = Vec::new();

	for mut item in items {
		match &mut item {
			Item::Fn(fn_item) => {
				if let Some(idx) =
					fn_item.attrs.iter().position(|a| a.path().is_ident("surrealism"))
				{
					let attr = fn_item.attrs.remove(idx);
					let (inner_default, inner_name, inner_init) = parse_surrealism_attr(&attr);

					if inner_init {
						panic!("#[surrealism(init)] cannot be used inside a module");
					}

					let export_name = if inner_default {
						prefix.to_string()
					} else {
						let base = inner_name.unwrap_or_else(|| fn_item.sig.ident.to_string());
						format!("{prefix}::{base}")
					};

					if export_name == "default" {
						panic!(
							"`default` is reserved for the default export; use \
							 #[surrealism(default)] on the function that should be \
							 the default export instead of naming it \"default\""
						);
					}

					let fn_name = &fn_item.sig.ident;
					let (arg_patterns, tuple_type, tuple_pattern, result_type, is_result) =
						match extract_fn_signature(&fn_item.sig) {
							Ok(v) => v,
							Err(e) => {
								new_items.push(Item::Verbatim(e.to_compile_error()));
								continue;
							}
						};

					sentinels.push(generate_sentinel(Some(&export_name)));

					let registration = generate_registration_body(
						fn_name,
						&arg_patterns,
						&tuple_type,
						&tuple_pattern,
						&result_type,
						is_result,
						false,
						Some(&export_name),
					);

					new_items.push(item);
					new_items.push(Item::Verbatim(registration));
					continue;
				}
			}
			Item::Mod(inner_mod) => {
				if let Some(idx) =
					inner_mod.attrs.iter().position(|a| a.path().is_ident("surrealism"))
				{
					let attr = inner_mod.attrs.remove(idx);
					let (inner_default, inner_name, inner_init) = parse_surrealism_attr(&attr);

					if inner_default {
						panic!("#[surrealism(default)] cannot be used on a module");
					}
					if inner_init {
						panic!("#[surrealism(init)] cannot be used on a module");
					}

					let inner_prefix_segment =
						inner_name.unwrap_or_else(|| inner_mod.ident.to_string());
					let inner_prefix = format!("{prefix}::{inner_prefix_segment}");

					let (brace, inner_items) = inner_mod.content.take().expect(
						"#[surrealism] on mod requires an inline module body (mod foo { ... })",
					);

					let (processed_items, inner_sentinels) =
						process_mod_items(&inner_prefix, inner_items);

					inner_mod.content = Some((brace, processed_items));
					sentinels.extend(inner_sentinels);

					new_items.push(item);
					continue;
				}
			}
			_ => {}
		}
		new_items.push(item);
	}

	(new_items, sentinels)
}

mod attr;
mod extract;
mod generate;
mod handler;

use attr::parse_surrealism_attrs;
use handler::{handle_function, handle_module};
use proc_macro::TokenStream;
use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{ItemFn, ItemMod, Meta, parse_macro_input};

#[proc_macro_attribute]
pub fn surrealism(attr: TokenStream, item: TokenStream) -> TokenStream {
	let args = parse_macro_input!(attr with Punctuated::<Meta, Comma>::parse_terminated);
	let (is_default, export_name_override, is_init, is_writeable, comment) =
		parse_surrealism_attrs(&args);

	let item2 = item.clone();
	if let Ok(item_fn) = syn::parse::<ItemFn>(item2) {
		handle_function(is_default, export_name_override, is_init, is_writeable, comment, item_fn)
	} else {
		let item_mod = parse_macro_input!(item as ItemMod);
		handle_module(is_default, export_name_override, is_init, is_writeable, comment, item_mod)
	}
}

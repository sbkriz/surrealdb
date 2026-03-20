use syn::punctuated::Punctuated;
use syn::token::Comma;
use syn::{Attribute, Expr, ExprLit, Lit, Meta, MetaNameValue};

/// Validate that an export name uses only valid segment characters separated by `::`.
pub(crate) fn validate_export_name(val: &str) {
	for segment in val.split("::") {
		if segment.is_empty() || !segment.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
			panic!(
				"#[surrealism(name = \"...\")] segments must use only ASCII \
				 letters, digits, and underscores, separated by `::`"
			);
		}
	}
}

/// Returns `(is_default, export_name_override, is_init, is_writeable)`.
pub(crate) fn parse_surrealism_attrs(
	args: &Punctuated<Meta, Comma>,
) -> (bool, Option<String>, bool, bool) {
	let mut is_default = false;
	let mut export_name_override: Option<String> = None;
	let mut is_init = false;
	let mut is_writeable = false;

	for meta in args.iter() {
		match meta {
			Meta::NameValue(MetaNameValue {
				path,
				value,
				..
			}) if path.is_ident("name") => {
				if let Expr::Lit(ExprLit {
					lit: Lit::Str(s),
					..
				}) = value
				{
					let val = s.value();
					validate_export_name(&val);
					export_name_override = Some(val);
				}
			}
			Meta::Path(path) if path.is_ident("default") => {
				is_default = true;
			}
			Meta::Path(path) if path.is_ident("init") => {
				is_init = true;
			}
			Meta::Path(path) if path.is_ident("writeable") => {
				is_writeable = true;
			}
			_ => panic!(
				"Unsupported attribute: expected #[surrealism], #[surrealism(default)], \
				 #[surrealism(init)], #[surrealism(writeable)], or #[surrealism(name = \"...\")]"
			),
		}
	}

	(is_default, export_name_override, is_init, is_writeable)
}

/// Parse surrealism attribute arguments from a `syn::Attribute` (used when
/// stripping inner attributes inside a mod).
pub(crate) fn parse_surrealism_attr(attr: &Attribute) -> (bool, Option<String>, bool, bool) {
	match &attr.meta {
		Meta::Path(_) => (false, None, false, false),
		Meta::List(list) => {
			let args: Punctuated<Meta, Comma> = list
				.parse_args_with(Punctuated::parse_terminated)
				.expect("failed to parse inner #[surrealism(...)] attribute");
			parse_surrealism_attrs(&args)
		}
		Meta::NameValue(_) => {
			panic!("#[surrealism] does not support top-level name = value syntax")
		}
	}
}

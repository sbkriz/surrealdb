mod fields;
pub use fields::*;

mod r#enum;
pub use r#enum::*;

mod with;
pub use with::*;

mod strategy;
pub use strategy::*;

/// Checks whether `ty` syntactically contains a path segment whose identifier
/// matches `ident`. Used by the derive macro to detect direct self-reference
/// in field types so that `kind_of()` can emit `Kind::Any` instead of recursing.
pub fn type_contains_ident(ty: &syn::Type, ident: &syn::Ident) -> bool {
	match ty {
		syn::Type::Path(type_path) => type_path.path.segments.iter().any(|seg| {
			if seg.ident == *ident {
				return true;
			}
			match &seg.arguments {
				syn::PathArguments::AngleBracketed(args) => args.args.iter().any(|arg| match arg {
					syn::GenericArgument::Type(inner) => type_contains_ident(inner, ident),
					_ => false,
				}),
				syn::PathArguments::Parenthesized(args) => {
					args.inputs.iter().any(|inner| type_contains_ident(inner, ident))
						|| matches!(&args.output, syn::ReturnType::Type(_, ret) if type_contains_ident(ret, ident))
				}
				syn::PathArguments::None => false,
			}
		}),
		syn::Type::Reference(r) => type_contains_ident(&r.elem, ident),
		syn::Type::Slice(s) => type_contains_ident(&s.elem, ident),
		syn::Type::Array(a) => type_contains_ident(&a.elem, ident),
		syn::Type::Tuple(t) => t.elems.iter().any(|el| type_contains_ident(el, ident)),
		syn::Type::Paren(p) => type_contains_ident(&p.elem, ident),
		syn::Type::Group(g) => type_contains_ident(&g.elem, ident),
		_ => false,
	}
}

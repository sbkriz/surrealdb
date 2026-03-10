"""Custom repository-specific wrappers for Bazel rules_rust."""

load(
    "@rules_rust//rust:defs.bzl",
    _rust_binary = "rust_binary",
    _rust_library = "rust_library",
    _rust_proc_macro = "rust_proc_macro",
    _rust_test = "rust_test",
)

def monorepo_crate_name(name):
    """Generate a crate name from the Bazel package path.

    Derives a crate name by combining the package path with the target name,
    replacing path separators and hyphens with underscores.

    Args:
        name: The target name.
    Returns:
        A valid Rust crate name.
    """
    if native.repository_name() not in ["@", "@{}".format(native.module_name())]:
        fail("monorepo_crate_name() is only supported in the root workspace")

    remove_invalid_chars = lambda s: s.replace("/", "_").replace("-", "_")

    crate_name = remove_invalid_chars(native.package_name())
    if not crate_name.endswith(name):
        crate_name += "_" + name

    return remove_invalid_chars(crate_name)

def get_rustc_flags(rustc_flags):
    """Build rustc flags, adding lint flags when the lint setting is enabled.

    Mirrors the [workspace.lints.clippy] configuration from Cargo.toml.

    Args:
        rustc_flags: Base flags to include unconditionally.
    Returns:
        A select()-based list of rustc flags.
    """
    return select({
        "@surrealdb//tools/settings:lint.enabled": [
            "-Wclippy::assigning_clones",
            "-Wclippy::cloned_instead_of_copied",
            "-Wclippy::debug_assert_with_mut_call",
            "-Wclippy::disallowed_methods",
            "-Wclippy::explicit_into_iter_loop",
            "-Wclippy::fallible_impl_from",
            "-Wclippy::get_unwrap",
            "-Wclippy::implicit_clone",
            "-Wclippy::inefficient_to_string",
            "-Wclippy::large_types_passed_by_value",
            "-Wclippy::lossy_float_literal",
            "-Wclippy::no_effect_underscore_binding",
            "-Wclippy::option_as_ref_cloned",
            "-Wclippy::redundant_clone",
            "-Wclippy::set_contains_or_insert",
            "-Wclippy::unnecessary_to_owned",
            "-Wclippy::unwrap_used",
            "-Wclippy::used_underscore_binding",
            "-Aclippy::allow_attributes",
            "-Aclippy::bool_assert_comparison",
            "-Aclippy::unused_async",
        ],
        "//conditions:default": [],
    }) + (rustc_flags if type(rustc_flags) == "select" else select({
        "//conditions:default": rustc_flags,
    }))

def rust_binary(name, srcs = [], crate_name = None, rustc_flags = [], **kwargs):
    """Rust binary rule with optional crate name generation and test support.

    Args:
        name: The name of the binary.
        srcs: The source files for the binary.
        crate_name: Override crate name. Defaults to monorepo_crate_name(name).
        rustc_flags: Additional flags to pass to rustc.
        **kwargs: Additional keyword arguments. Keys prefixed with test_ are
            forwarded to the companion test target.
    """
    if not srcs:
        srcs = ["{}.rs".format(name)]

    if not crate_name:
        crate_name = monorepo_crate_name(name)

    kwargs, test_kwargs = _split_test_kwargs(kwargs)

    _rust_binary(
        name = name,
        crate_name = crate_name,
        srcs = srcs,
        rustc_flags = get_rustc_flags(rustc_flags),
        **kwargs
    )

    rust_test(
        name = name + ".test",
        crate = name,
        **test_kwargs
    )

def rust_library(name, srcs = [], crate_name = None, rustc_flags = [], **kwargs):
    """Rust library rule with optional crate name generation and test support.

    Args:
        name: The name of the library.
        srcs: The source files for the library.
        crate_name: Override crate name. Defaults to monorepo_crate_name(name).
        rustc_flags: Additional flags to pass to rustc.
        **kwargs: Additional keyword arguments. Keys prefixed with test_ are
            forwarded to the companion test target.
    """
    if not srcs:
        srcs = native.glob(["**/*.rs"])

    if not crate_name:
        crate_name = monorepo_crate_name(name)

    kwargs, test_kwargs = _split_test_kwargs(kwargs)

    _rust_library(
        name = name,
        crate_name = crate_name,
        srcs = srcs,
        rustc_flags = get_rustc_flags(rustc_flags),
        **kwargs
    )

    rust_test(
        name = name + ".test",
        crate = name,
        **test_kwargs
    )

def rust_proc_macro(name, srcs = [], crate_name = None, rustc_flags = [], **kwargs):
    """Rust proc-macro rule with optional crate name generation and test support.

    Args:
        name: The name of the procedural macro.
        srcs: The source files for the procedural macro.
        crate_name: Override crate name. Defaults to monorepo_crate_name(name).
        rustc_flags: Additional flags to pass to rustc.
        **kwargs: Additional keyword arguments. Keys prefixed with test_ are
            forwarded to the companion test target.
    """
    if not srcs:
        srcs = native.glob(["**/*.rs"])

    if not crate_name:
        crate_name = monorepo_crate_name(name)

    kwargs, test_kwargs = _split_test_kwargs(kwargs)

    _rust_proc_macro(
        name = name,
        crate_name = crate_name,
        srcs = srcs,
        rustc_flags = get_rustc_flags(rustc_flags),
        **kwargs
    )

    rust_test(
        name = name + ".test",
        crate = name,
        **test_kwargs
    )

def rust_test(name, crate, srcs = None, rustc_flags = [], **kwargs):
    """Rust test rule wrapper.

    Args:
        name: The name of the test target.
        crate: The crate target to test.
        srcs: Optional source files.
        rustc_flags: Additional flags to pass to rustc.
        **kwargs: Additional keyword arguments.
    """
    _rust_test(
        name = name,
        crate = crate,
        srcs = srcs,
        rustc_flags = get_rustc_flags(rustc_flags),
        testonly = True,
        **kwargs
    )

def _split_test_kwargs(kwargs):
    """Split test-related kwargs from the rest.

    Keys prefixed with "test_" are collected into a separate dict with the
    prefix stripped (e.g. test_deps -> deps). The "tags" key is shared
    between both dicts so that tags like "manual" propagate to the
    companion test target.

    Args:
        kwargs: The keyword arguments to split.
    Returns:
        A tuple of (non-test kwargs, test kwargs).
    """
    out_kwargs = {}
    out_test_kwargs = {}
    for key, value in kwargs.items():
        if key.startswith("test_"):
            out_test_kwargs[key[5:]] = value
        else:
            out_kwargs[key] = value

    if "tags" in out_kwargs and "tags" not in out_test_kwargs:
        out_test_kwargs["tags"] = out_kwargs["tags"]

    return out_kwargs, out_test_kwargs

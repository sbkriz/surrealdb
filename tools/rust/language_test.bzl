"""Bazel rules for SurrealQL language tests.

Each .surql test file becomes one Bazel test target per backend, plus a
test_suite that groups all backends for that file.

Uses Bazel persistent workers to keep bootstrapped Datastore instances
alive across test invocations, amortising the per-test startup cost.
Each test is a two-phase rule:
  1. Build phase: an action runs the test via persistent worker, producing
     a result file (PASS/FAIL + diagnostic output).
  2. Test phase: a thin shell script reads the result file and exits 0/1.

Imports are handled via a config dict: each entry maps a test file path
to its imports and overrides. Files not in the dict have no imports and
only their own .surql file as data.

Config transitions rebuild the runner binary with the correct storage
engine features for each backend.
"""

load("//tools/rust:transitions.bzl", "make_surrealdb_transition")

_lang_mem_transition = make_surrealdb_transition(storage = "mem", scripting = True)
_lang_rocksdb_transition = make_surrealdb_transition(storage = "rocksdb", scripting = True)
_lang_surrealkv_transition = make_surrealdb_transition(storage = "surrealkv", scripting = True)
_lang_tikv_transition = make_surrealdb_transition(storage = "tikv", scripting = True)

def _surrealql_test_impl(ctx):
    """Run a single .surql test via persistent worker, check result in test phase."""
    binary = ctx.executable.test_binary
    result_file = ctx.actions.declare_file(ctx.label.name + ".result")

    # Phase 1: run the test via persistent worker (or local fallback).
    # Workers require arguments to be passed via a flagfile.
    args_file = ctx.actions.declare_file(ctx.label.name + ".args")
    ctx.actions.write(
        output = args_file,
        content = "\n".join([
            "--backend", ctx.attr.backend,
            "--test-root", ctx.attr.test_root,
            "--test-file", ctx.attr.test_file,
            "--result-file", result_file.path,
        ]),
    )

    mnemonic = "SurrealQLTest"
    exec_reqs = {
        "supports-workers": "1",
        "supports-multiplex-workers": "1",
        "requires-worker-protocol": "json",
    }
    if ctx.attr.backend == "tikv":
        exec_reqs["requires-network"] = "1"
        # TiKV tests share a single playground cluster, so they must run
        # one at a time.  Use a dedicated mnemonic (controlled separately
        # in .bazelrc) and disable multiplex to serialise requests.
        exec_reqs["supports-multiplex-workers"] = "0"
        mnemonic = "SurrealQLTestTikV"

    ctx.actions.run(
        mnemonic = mnemonic,
        executable = binary,
        inputs = ctx.files.test_data + [args_file],
        outputs = [result_file],
        arguments = ["@" + args_file.path],
        execution_requirements = exec_reqs,
    )

    # Phase 2: thin test wrapper that checks the result file.
    runner = ctx.actions.declare_file(ctx.label.name + "_runner.sh")
    ctx.actions.write(
        output = runner,
        content = """#!/usr/bin/env bash
set -euo pipefail
RESULT="{result}"
STATUS=$(head -1 "$RESULT")
BODY=$(tail -n +2 "$RESULT")
if [ -n "$BODY" ]; then
  echo "$BODY"
fi
if [ "$STATUS" = "PASS" ]; then
  exit 0
else
  exit 1
fi
""".format(result = result_file.short_path),
        is_executable = True,
    )

    runfiles = ctx.runfiles(files = [result_file])
    return [DefaultInfo(
        executable = runner,
        runfiles = runfiles,
    )]

def _make_surrealql_test_rule(backend_transition):
    """Create a test rule for a specific backend transition."""
    return rule(
        implementation = _surrealql_test_impl,
        cfg = backend_transition,
        attrs = {
            "test_binary": attr.label(
                mandatory = True,
                executable = True,
                cfg = "target",
            ),
            "test_file": attr.string(mandatory = True),
            "backend": attr.string(mandatory = True),
            "test_root": attr.string(default = "tests"),
            "test_data": attr.label_list(allow_files = True),
            "_allowlist_function_transition": attr.label(
                default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
            ),
        },
        test = True,
    )

_surrealql_mem_test = _make_surrealql_test_rule(_lang_mem_transition)
_surrealql_rocksdb_test = _make_surrealql_test_rule(_lang_rocksdb_transition)
_surrealql_surrealkv_test = _make_surrealql_test_rule(_lang_surrealkv_transition)
_surrealql_tikv_test = _make_surrealql_test_rule(_lang_tikv_transition)

_BACKEND_RULES = {
    "mem": _surrealql_mem_test,
    "rocksdb": _surrealql_rocksdb_test,
    "surrealkv": _surrealql_surrealkv_test,
    "tikv": _surrealql_tikv_test,
}

def surrealql_test(
        name,
        test_file,
        backend,
        test_binary,
        test_data = [],
        test_root = "tests",
        tags = [],
        size = "small",
        timeout = "short",
        **kwargs):
    """Create a Bazel test target for a single .surql test on a specific backend.

    Args:
        name: Target name.
        test_file: Path to the .surql file relative to the test root.
        backend: Storage backend ("mem", "rocksdb", "surrealkv", "tikv").
        test_binary: Label of the surrealql-test binary.
        test_data: Data files (the test .surql + its imports).
        test_root: Path to the test root directory in runfiles.
        tags: Additional tags.
        size: Test size.
        timeout: Test timeout.
        **kwargs: Extra kwargs.
    """
    rule_fn = _BACKEND_RULES.get(backend)
    if not rule_fn:
        fail("Unknown backend: " + backend)

    backend_tags = list(tags) + [backend, "surrealql"]
    if backend == "tikv":
        backend_tags.append("external")

    rule_fn(
        name = name,
        test_binary = test_binary,
        test_file = test_file,
        backend = backend,
        test_root = test_root,
        test_data = test_data,
        tags = backend_tags,
        size = size,
        timeout = timeout,
        **kwargs
    )

def surrealql_test_suite(
        name,
        test_files,
        backends,
        test_binary,
        test_root = "tests",
        config = {},
        tags = [],
        size = "small",
        timeout = "short",
        **kwargs):
    """Generate Bazel test targets for SurrealQL language tests.

    For each .surql file, creates one test target per backend plus a
    test_suite that groups all backends for that file. Only the test
    file itself and its declared imports are included as data deps.

    Args:
        name: Suite name prefix.
        test_files: List of .surql file paths (from glob, relative to package).
        backends: List of backend names to test against.
        test_binary: Label of the surrealql-test binary.
        test_root: Directory containing tests, relative to the package.
        config: Dict mapping test file paths (relative to test_root, e.g.
            "language/graph/path_shortest.surql") to config dicts. Supported
            keys:
              - "imports": list of import file paths relative to test_root
              - "exclude_backends": list of backends to skip
              - "skip": if True, skip this test entirely (no targets generated)
              - "data": list of extra data file paths relative to test_root
              - "size": override test size
              - "timeout": override test timeout
              - "tags": additional tags
            Files not in config have no imports and use suite-level defaults.
        tags: Additional tags for all targets.
        size: Default test size.
        timeout: Default test timeout.
        **kwargs: Extra kwargs forwarded to each test target.
    """

    runtime_root = native.package_name() + "/" + test_root

    for f in test_files:
        # "tests/language/graph/basic.surql" -> "language/graph/basic.surql"
        rel = f
        if rel.startswith(test_root + "/"):
            rel = rel[len(test_root) + 1:]

        short = rel
        if short.endswith(".surql"):
            short = short[:-len(".surql")]
        sanitized = short.replace("/", "_").replace("-", "_").replace(".", "_")

        # Look up per-test config
        file_config = config.get(rel, {})

        if file_config.get("skip", False):
            continue

        imports = file_config.get("imports", [])
        excluded_backends = file_config.get("exclude_backends", [])
        extra_data = file_config.get("data", [])
        test_size = file_config.get("size", size)
        test_timeout = file_config.get("timeout", timeout)
        extra_tags = file_config.get("tags", [])

        # Data deps: the test file itself + its declared imports + extra data files
        test_data = [test_root + "/" + rel]
        for imp in imports:
            test_data.append(test_root + "/" + imp)
        for d in extra_data:
            test_data.append(test_root + "/" + d)

        backend_targets = []
        for backend in backends:
            if backend in excluded_backends:
                continue

            target_name = "{}_{}.{}".format(name, sanitized, backend)
            surrealql_test(
                name = target_name,
                test_file = rel,
                backend = backend,
                test_binary = test_binary,
                test_data = test_data,
                test_root = runtime_root,
                tags = tags + extra_tags,
                size = test_size,
                timeout = test_timeout,
                **kwargs
            )
            backend_targets.append(":" + target_name)

        # test_suite grouping all backends for this file
        if backend_targets:
            suite_name = "{}_{}".format(name, sanitized)
            native.test_suite(
                name = suite_name,
                tests = backend_targets,
                tags = ["surrealql"] + tags + extra_tags,
            )

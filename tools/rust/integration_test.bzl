"""Custom test rules with config transitions for SurrealDB CI profiles.

surrealdb_test: For embedded/in-process tests (workspace tests, KVS, api-mem).
surrealdb_server_test: For tests needing a running server (cli, http, ws).

Both rules apply an incoming-edge transition so the entire dependency tree
is rebuilt with the feature configuration the test requires.

Rules are pre-created at .bzl top level for each known transition.
Bazel requires rule() to be called during top-level .bzl evaluation and
each rule must be assigned to a top-level variable to be "exported".
"""

load(
    "//tools/rust:transitions.bzl",
    "cli_integration_transition",
    "graphql_integration_transition",
    "http_integration_transition",
    "ws_integration_transition",
    "workspace_test_transition",
    "kvs_mem_transition",
    "kvs_rocksdb_transition",
    "kvs_surrealkv_transition",
    "kvs_tikv_transition",
    "api_mem_transition",
    "api_ws_transition",
    "api_http_transition",
)

# ---------------------------------------------------------------------------
# surrealdb_test: wraps a rust_test with a config transition
# ---------------------------------------------------------------------------

def _forward_transition_test_impl(ctx):
    inner_executable = ctx.executable.inner_test
    script = ctx.actions.declare_file(ctx.label.name + "_runner.sh")
    ctx.actions.write(
        output = script,
        content = '#!/usr/bin/env bash\nexec "{}" "$@"\n'.format(
            inner_executable.short_path,
        ),
        is_executable = True,
    )
    runfiles = ctx.runfiles(files = [inner_executable])
    runfiles = runfiles.merge(ctx.attr.inner_test[DefaultInfo].default_runfiles)
    return [DefaultInfo(executable = script, runfiles = runfiles)]

def _make_test_rule(transition_fn):
    return rule(
        implementation = _forward_transition_test_impl,
        cfg = transition_fn,
        attrs = {
            "inner_test": attr.label(
                mandatory = True,
                executable = True,
                cfg = "target",
            ),
            "_allowlist_function_transition": attr.label(
                default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
            ),
        },
        test = True,
    )

# Each rule must be a named top-level variable ending in _test to be exported.
_cli_integration_test = _make_test_rule(cli_integration_transition)
_http_integration_test = _make_test_rule(http_integration_transition)
_ws_integration_test = _make_test_rule(ws_integration_transition)
_graphql_integration_test = _make_test_rule(graphql_integration_transition)
_workspace_test_test = _make_test_rule(workspace_test_transition)
_kvs_mem_test = _make_test_rule(kvs_mem_transition)
_kvs_rocksdb_test = _make_test_rule(kvs_rocksdb_transition)
_kvs_surrealkv_test = _make_test_rule(kvs_surrealkv_transition)
_kvs_tikv_test = _make_test_rule(kvs_tikv_transition)
_api_mem_test = _make_test_rule(api_mem_transition)
_api_ws_test = _make_test_rule(api_ws_transition)
_api_http_test = _make_test_rule(api_http_transition)

_TEST_RULES = {
    "cli_integration": _cli_integration_test,
    "http_integration": _http_integration_test,
    "ws_integration": _ws_integration_test,
    "graphql_integration": _graphql_integration_test,
    "workspace_test": _workspace_test_test,
    "kvs_mem": _kvs_mem_test,
    "kvs_rocksdb": _kvs_rocksdb_test,
    "kvs_surrealkv": _kvs_surrealkv_test,
    "kvs_tikv": _kvs_tikv_test,
    "api_mem": _api_mem_test,
    "api_ws": _api_ws_test,
    "api_http": _api_http_test,
}

def surrealdb_test(name, transition, inner_test, **kwargs):
    """Macro that wraps an existing rust_test target with a config transition.

    Args:
        name: Name for the transitioned test target.
        transition: Name (string) of a transition from transitions.bzl, e.g.
            "graphql_integration". Must be a key in _TEST_RULES.
        inner_test: Label of the rust_test target to wrap.
        **kwargs: Forwarded to the wrapper rule (tags, size, timeout, etc.).
    """
    rule_fn = _TEST_RULES.get(transition)
    if not rule_fn:
        fail("Unknown transition '{}'. Known: {}".format(
            transition,
            ", ".join(_TEST_RULES.keys()),
        ))
    rule_fn(
        name = name,
        inner_test = inner_test,
        **kwargs
    )

# ---------------------------------------------------------------------------
# surrealdb_server_test: starts a server, then runs a test binary
# ---------------------------------------------------------------------------

def _server_test_impl(ctx):
    server = ctx.executable.server
    test_bin = ctx.executable.test_binary

    script = ctx.actions.declare_file(ctx.label.name + "_runner.sh")
    ctx.actions.write(
        output = script,
        content = """#!/usr/bin/env bash
set -euo pipefail

SERVER="{server}"
TEST="{test}"
PORT="${{SURREAL_PORT:-18000}}"

cleanup() {{
    if [ -n "${{SERVER_PID:-}}" ]; then
        kill "$SERVER_PID" 2>/dev/null || true
        wait "$SERVER_PID" 2>/dev/null || true
    fi
}}
trap cleanup EXIT

"$SERVER" start --bind "0.0.0.0:$PORT" --allow-all -u root -p root memory &
SERVER_PID=$!

for i in $(seq 1 30); do
    if curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.5
done

if ! curl -sf "http://127.0.0.1:$PORT/health" >/dev/null 2>&1; then
    echo "Server failed to start" >&2
    exit 1
fi

export SURREAL_PORT="$PORT"
export SURREAL_BIN="$(cd "$(dirname "$SERVER")" && pwd)/$(basename "$SERVER")"
"$TEST" "$@"
""".format(
            server = server.short_path,
            test = test_bin.short_path,
        ),
        is_executable = True,
    )

    runfiles = ctx.runfiles(files = [server, test_bin])
    runfiles = runfiles.merge(ctx.attr.server[DefaultInfo].default_runfiles)
    runfiles = runfiles.merge(ctx.attr.test_binary[DefaultInfo].default_runfiles)

    return [DefaultInfo(
        executable = script,
        runfiles = runfiles,
    )]

def _make_server_test_rule(transition_fn):
    return rule(
        implementation = _server_test_impl,
        cfg = transition_fn,
        attrs = {
            "server": attr.label(
                mandatory = True,
                executable = True,
                cfg = "target",
            ),
            "test_binary": attr.label(
                mandatory = True,
                executable = True,
                cfg = "target",
            ),
            "_allowlist_function_transition": attr.label(
                default = "@bazel_tools//tools/allowlists/function_transition_allowlist",
            ),
        },
        test = True,
    )

_server_cli_integration_test = _make_server_test_rule(cli_integration_transition)
_server_http_integration_test = _make_server_test_rule(http_integration_transition)
_server_ws_integration_test = _make_server_test_rule(ws_integration_transition)
_server_graphql_integration_test = _make_server_test_rule(graphql_integration_transition)
_server_workspace_test_test = _make_server_test_rule(workspace_test_transition)

_SERVER_TEST_RULES = {
    "cli_integration": _server_cli_integration_test,
    "http_integration": _server_http_integration_test,
    "ws_integration": _server_ws_integration_test,
    "graphql_integration": _server_graphql_integration_test,
    "workspace_test": _server_workspace_test_test,
}

def surrealdb_server_test(
        name,
        transition,
        server,
        test_binary,
        tags = [],
        size = "large",
        timeout = "moderate",
        **kwargs):
    """Macro for tests that need a running SurrealDB server.

    Args:
        name: Test target name.
        transition: Name (string) of a transition from transitions.bzl, e.g.
            "cli_integration". Must be a key in _SERVER_TEST_RULES.
        server: Label of the surreal binary.
        test_binary: Label of the compiled test binary.
        tags: Test tags.
        size: Test size (default "large" since server tests are slow).
        timeout: Test timeout.
        **kwargs: Additional kwargs forwarded to the rule.
    """
    rule_fn = _SERVER_TEST_RULES.get(transition)
    if not rule_fn:
        fail("Unknown transition '{}'. Known: {}".format(
            transition,
            ", ".join(_SERVER_TEST_RULES.keys()),
        ))
    rule_fn(
        name = name,
        server = server,
        test_binary = test_binary,
        tags = tags,
        size = size,
        timeout = timeout,
        **kwargs
    )

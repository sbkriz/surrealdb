"""Config transitions for SurrealDB CI test profiles.

Each transition sets the build settings defined in //tools/settings to match
a specific CI test configuration (feature set + storage engine). Test rules
apply these as incoming edge transitions so that `bazel test //...` builds
each test's dependency chain with the right features.
"""

_SETTINGS = "//tools/settings"

_ALL_OUTPUTS = [
    _SETTINGS + ":storage_engine",
    _SETTINGS + ":feature_http",
    _SETTINGS + ":feature_scripting",
    _SETTINGS + ":feature_graphql",
    _SETTINGS + ":feature_jwks",
    _SETTINGS + ":feature_cli",
    _SETTINGS + ":feature_surrealism",
    _SETTINGS + ":protocol_http",
    _SETTINGS + ":protocol_ws",
]

def _make_transition_impl(settings_dict):
    def _impl(settings, attr):
        _ignore = (settings, attr)
        return settings_dict

    return _impl

def make_surrealdb_transition(
        storage = "mem",
        http = False,
        scripting = False,
        graphql = False,
        jwks = False,
        cli = False,
        surrealism = False,
        proto_http = False,
        proto_ws = False):
    """Create a config transition that sets SurrealDB build settings.

    Args:
        storage: Storage engine ("mem", "rocksdb", "surrealkv", "tikv").
        http: Enable the http feature on surrealdb-core/server.
        scripting: Enable the scripting (JS) feature.
        graphql: Enable GraphQL support.
        jwks: Enable JWKS support.
        cli: Enable the interactive CLI.
        surrealism: Enable the surrealism WASM plugin system.
        proto_http: Enable the HTTP protocol on the SDK.
        proto_ws: Enable the WebSocket protocol on the SDK.
    Returns:
        A transition object.
    """
    settings_dict = {
        _SETTINGS + ":storage_engine": storage,
        _SETTINGS + ":feature_http": http,
        _SETTINGS + ":feature_scripting": scripting,
        _SETTINGS + ":feature_graphql": graphql,
        _SETTINGS + ":feature_jwks": jwks,
        _SETTINGS + ":feature_cli": cli,
        _SETTINGS + ":feature_surrealism": surrealism,
        _SETTINGS + ":protocol_http": proto_http,
        _SETTINGS + ":protocol_ws": proto_ws,
    }
    return transition(
        implementation = _make_transition_impl(settings_dict),
        inputs = [],
        outputs = _ALL_OUTPUTS,
    )

# ---------------------------------------------------------------------------
# CI test profile transitions
# ---------------------------------------------------------------------------

cli_integration_transition = make_surrealdb_transition(
    storage = "mem",
    http = True,
    scripting = True,
    jwks = True,
    cli = True,
)

http_integration_transition = make_surrealdb_transition(
    storage = "mem",
    jwks = True,
)

ws_integration_transition = make_surrealdb_transition(
    storage = "mem",
)

graphql_integration_transition = make_surrealdb_transition(
    storage = "mem",
    graphql = True,
)

workspace_test_transition = make_surrealdb_transition(
    storage = "mem",
    http = True,
    scripting = True,
    jwks = True,
)

kvs_mem_transition = make_surrealdb_transition(storage = "mem")
kvs_rocksdb_transition = make_surrealdb_transition(storage = "rocksdb")
kvs_surrealkv_transition = make_surrealdb_transition(storage = "surrealkv")
kvs_tikv_transition = make_surrealdb_transition(storage = "tikv")

api_mem_transition = make_surrealdb_transition(
    storage = "mem",
    proto_http = True,
    proto_ws = True,
)

api_ws_transition = make_surrealdb_transition(
    proto_ws = True,
)

api_http_transition = make_surrealdb_transition(
    proto_http = True,
)

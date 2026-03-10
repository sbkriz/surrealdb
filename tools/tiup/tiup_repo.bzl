"""Repository rule that fetches the tiup binary for the host platform.

tiup is used by the language-test worker to start a TiKV playground
(PD + TiKV) on demand.  The binary is downloaded from the official
PingCAP mirror and exposed as `@tiup//:tiup`.
"""

def _tiup_repo_impl(ctx):
    if ctx.os.name.startswith("mac"):
        os = "darwin"
    else:
        os = "linux"

    if ctx.os.arch in ("aarch64", "arm64"):
        arch = "arm64"
    else:
        arch = "amd64"

    url = "https://tiup-mirrors.pingcap.com/tiup-{}-{}.tar.gz".format(os, arch)
    ctx.download_and_extract(url = url, output = ".")

    # Ensure the binary is executable (tarballs from the mirror may lack +x).
    ctx.execute(["chmod", "+x", "tiup"])

    ctx.file("BUILD.bazel", content = """\
exports_files(["tiup"], visibility = ["//visibility:public"])
""")

tiup_repo = repository_rule(
    implementation = _tiup_repo_impl,
    doc = "Downloads the tiup binary for the host platform from the PingCAP mirror.",
)

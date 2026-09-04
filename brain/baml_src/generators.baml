// What `baml-cli generate` writes, and where.
//
// `baml_client/` is generated, never edited and never committed — it is in the root
// `.gitignore`, so anything that imports it only works after a `generate`. That is why
// nothing under `src/graphify_brain/` imports it yet: `cost.py` and `db.py` have to run
// in CI, which installs the package and runs pytest.

generator python {
    output_type "python/pydantic"

    // Relative to `baml_src/`, so the package lands at `brain/baml_client/`.
    output_dir "../"

    // Must match the installed `baml-py`. `uv.lock` pins 0.226.2; if that moves, this
    // moves with it or `generate` refuses.
    version "0.226.2"

    // The brain is a CLI: one call, one answer, exit. Nothing here is concurrent, so
    // async would buy an event loop and no speed.
    default_client_mode sync
}

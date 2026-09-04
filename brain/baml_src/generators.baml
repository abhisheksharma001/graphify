// What `baml-cli generate` writes, and where.
//
// `baml_client/` is generated, never edited and never committed — it is in the root
// `.gitignore`, so anything that imports it only works after a `generate`. `plan.py` is
// the first module that needs it, and it does the import inside the function rather than
// at module scope, so `graphify-brain version` still runs on a fresh checkout.

generator python {
    output_type "python/pydantic"

    // Relative to `baml_src/`, so the package lands at `brain/src/baml_client/` —
    // beside `graphify_brain` and inside the one directory the editable install puts on
    // `sys.path`. Generating it into `brain/` instead would make it importable only from
    // a process whose working directory happened to be `brain/`, and the engine spawns
    // this program from wherever it was started.
    output_dir "../src/"

    // Must match the installed `baml-py`. `uv.lock` pins 0.226.2; if that moves, this
    // moves with it or `generate` refuses.
    version "0.226.2"

    // The brain is a CLI: one call, one answer, exit. Nothing here is concurrent, so
    // async would buy an event loop and no speed.
    default_client_mode sync
}

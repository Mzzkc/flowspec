# Source-bound normal builds

When normal builds share a Cargo target, use `python3 scripts/build-normal.py
--lease /path/to/shared.lock --output /path/outside/checkout/flowspec-normal`.
The recipe derives a clean checkout's root, HEAD and tree and passes their value
as FLOWSPEC_BUILD_SOURCE_SEAL. build.rs tracks and validates that value, so a
worktree switch invalidates the generated producer without timestamp touching.
The same lease covers Cargo metadata, normal compilation and immutable copying.
The adjacent binding receipt identifies source and copied bytes; consume the CLI
report's authority.producer and compare build_sha/source_tree_sha before assigning
consumer evidence. Compilation alone is insufficient. Old source without the seal
hook does not gain this guarantee by running the newer wrapper.

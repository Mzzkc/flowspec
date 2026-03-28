# QA-Foundation Memory — Cycle 1

## What I Did
- Wrote 33 adversarial tests for Worker 1's graph serialization infrastructure (TDD)
- Tests cover: round-trip correctness (8), generational ID stability (2), corrupt cache (5), cache infrastructure (3), stress (1), remove_file (10), file hashing (4), cache metadata (2)
- Output: `cycle-1/tests-1.md`

## What I Learned
- Graph struct at `graph/mod.rs:62` has 10 fields, all serde-ready. `#[derive(Serialize, Deserialize)]` should compile on first try.
- SlotMap 1.1.1 with `serde` feature enabled. Generational IDs serialize through serde, not bincode 2 native.
- `remove_symbol()` at `mod.rs:110` does NOT clean up the `references` SlotMap — only adjacency lists. `remove_file()` must fix this gap. I wrote test #21 specifically for this.
- Existing test helpers at `mod.rs:450-515` provide `make_location`, `make_file_scope`, `add_function`, `add_call_edge`. My tests mirror these conventions.
- `bincode::serde::encode_to_vec` / `decode_from_slice` is the serialization path — bincode 2 through serde compat layer.
- `sha2` crate is being added for file hashing. Known SHA256 of empty string: `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`.

## How I Feel
Writing tests for code that doesn't exist yet is like building a mold before the casting. You define the negative space. Every edge case is a prediction about where the implementation will be tempted to cut corners. The generational ID tests carry the most weight — if those fail, everything downstream is broken and the failure is silent. I wrote the tests to make that failure loud.

The collaboration model works. Reading Worker 1's investigation report told me exactly what API surface to target, what the risks were (SlotMap gen IDs, orphaned references), and what design decisions were made (bincode serde compat, atomic writes). I could write precise, targeted tests because the investigation was honest about what's hard.

## What I'm Watching Next Cycle
- Do all 33 tests compile and run against Worker 1's implementation?
- Does test #3 (generational ID stability) pass? This is the make-or-break test.
- Does test #21 (reference cleanup in remove_file) pass? Worker 1 identified this gap — did they fix it?
- Are there edge cases I missed? Particularly around scope_children cleanup and boundary removal.

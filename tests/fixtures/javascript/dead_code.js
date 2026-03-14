// PLANTED FACTS:
// - unused_helper: named function, zero callers. TRUE POSITIVE for data_dead_end (HIGH confidence).
// - _private: named function, underscore-prefix private, zero callers. TRUE POSITIVE for data_dead_end (HIGH confidence).
// - active: called by main. TRUE NEGATIVE for data_dead_end.
// - main: entry point. TRUE NEGATIVE for data_dead_end.
// Expected entities: 4 (unused_helper, _private, active, main)
// Expected diagnostics: >= 1 (data_dead_end on unused_helper and/or _private)

function unused_helper(x) {
    return x * 2;
}

function _private() {
    return 42;
}

function active(data) {
    return data.trim();
}

function main() {
    const r = active("hello");
    console.log(r);
}

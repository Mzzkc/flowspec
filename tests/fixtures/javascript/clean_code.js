// PLANTED FACTS:
// - All functions are called. Module is connected.
// - ZERO diagnostics expected from ANY pattern.
// Expected entities: 3 (read, transform, main)
// Expected diagnostics: 0

function read(path) {
    return path;
}

function transform(raw) {
    return read(raw).split("\n");
}

function main() {
    return transform("input.txt");
}

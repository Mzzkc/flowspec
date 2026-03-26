class Handler {
    process(input) {
        return this.sanitize(input);
    }

    sanitize(input) {
        return input.trim();
    }
}

function main() {
    const h = new Handler();
    h.process("  test  ");
}

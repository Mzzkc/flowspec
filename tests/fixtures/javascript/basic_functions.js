// PLANTED FACTS:
// - greet: named function, called by main. TRUE NEGATIVE for data_dead_end.
// - add: arrow function (const), called by main. TRUE NEGATIVE for data_dead_end.
// - multiply: named function, called by main. TRUE NEGATIVE for data_dead_end.
// - main: entry point, calls greet/add/multiply. TRUE NEGATIVE for data_dead_end.
// Expected entities: 4 (greet, add, multiply, main)
// Expected diagnostics: 0

function greet(name) {
    return `Hello, ${name}`;
}

const add = (a, b) => a + b;

function multiply(x, y) {
    return x * y;
}

function main() {
    console.log(greet("world"));
    console.log(add(1, 2));
    multiply(3, 4);
}

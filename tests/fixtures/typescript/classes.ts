// TypeScript classes fixture
// Tests: strip_ts_line_syntax() full pipeline, strip_leading_keyword(),
// strip_implements_clause(), entity dedup correctness

interface Config {
  debug: boolean;
}

interface Renderable {
  render(): void;
}

class App implements Config {
  debug: boolean = false;
  constructor() {}
  start() {}
}

class Derived extends Base implements Config {
  debug: boolean = true;
  constructor() {
    super();
  }
}

abstract class Shape {
  abstract area(): number;
  describe() {
    return "shape";
  }
}

export class Widget implements Renderable {
  render() {}
  update() {}
}

class MultiImpl implements Config, Renderable {
  debug: boolean = false;
  render() {}
}

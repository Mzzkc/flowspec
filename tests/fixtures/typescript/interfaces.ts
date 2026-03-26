// TypeScript interfaces and type aliases fixture
// Tests: detect_ts_block_start(), pre-extraction via try_extract_ts_entity(),
// blank-line byte offset preservation

interface User {
  name: string;
  age: number;
  email?: string;
}

interface Admin extends User {
  role: string;
  permissions: string[];
}

interface Collection<T> {
  items: T[];
  add(item: T): void;
  get(index: number): T;
}

type UserId = string | number;

type EventHandler = (event: Event) => void;

interface Logger {
  log(message: string): void;
  error(message: string): void;
}

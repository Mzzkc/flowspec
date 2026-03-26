// Mixed TypeScript fixture
// Tests: is_typescript_file() routing, full pipeline integration,
// entity dedup correctness across all TS constructs

import { EventEmitter } from "events";

interface Serializable {
  serialize(): string;
}

type Status = "active" | "inactive" | "pending";

enum Priority {
  Low,
  Medium,
  High,
}

function createLogger(name: string): Logger {
  return { log: console.log, error: console.error };
}

export function formatStatus(s: Status): string {
  return s.toUpperCase();
}

class DataStore<T> implements Serializable {
  private items: T[] = [];
  serialize() {
    return JSON.stringify(this.items);
  }
  add(item: T) {
    this.items.push(item);
  }
}

export default class Application {
  constructor() {}
  run() {}
}

const VERSION = "1.0.0";

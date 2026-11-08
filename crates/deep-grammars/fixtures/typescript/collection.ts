export class LinkedList<T> implements Iterable<T> {
  private head: { value: T; next: unknown } | null = null;
  private tail: { value: T; next: unknown } | null = null;
  private count = 0;

  get length(): number {
    return this.count;
  }

  push(value: T): this {
    const node = { value, next: null as unknown };
    if (this.tail === null) {
      this.head = node;
    } else {
      this.tail.next = node;
    }
    this.tail = node;
    this.count += 1;
    return this;
  }

  shift(): T | undefined {
    const node = this.head;
    if (node === null) return undefined;
    this.head = node.next as { value: T; next: unknown } | null;
    if (this.head === null) this.tail = null;
    this.count -= 1;
    return node.value;
  }

  *[Symbol.iterator](): Iterator<T> {
    let node = this.head;
    while (node !== null) {
      yield node.value;
      node = node.next as { value: T; next: unknown } | null;
    }
  }

  toArray(): T[] {
    const out: T[] = [];
    for (const value of this) out.push(value);
    return out;
  }

  static from<T>(items: Iterable<T>): LinkedList<T> {
    const list = new LinkedList<T>();
    for (const item of items) list.push(item);
    return list;
  }
}

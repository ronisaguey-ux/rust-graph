export type Attributes = Record<string, string | number | boolean>;

export interface Span {
  readonly name: string;
  readonly startedAt: number;
  end(attributes?: Attributes): void;
  child(name: string): Span;
}

export interface Exporter {
  export(spans: readonly FinishedSpan[]): Promise<void>;
}

export interface FinishedSpan {
  name: string;
  durationMs: number;
  attributes: Attributes;
  children: FinishedSpan[];
}

class SpanNode implements Span {
  readonly children: SpanNode[] = [];
  private done = false;

  constructor(
    readonly name: string,
    readonly startedAt: number,
    readonly attributes: Attributes,
  ) {}

  end(extra: Attributes = {}): void {
    if (this.done) throw new Error(`span ${this.name} already ended`);
    this.done = true;
    Object.assign(this.attributes, extra);
  }

  child(name: string): Span {
    const node = new SpanNode(name, Date.now(), {});
    this.children.push(node);
    return node;
  }

  finish(): FinishedSpan {
    return {
      name: this.name,
      durationMs: Date.now() - this.startedAt,
      attributes: { ...this.attributes },
      children: this.children.map((child) => child.finish()),
    };
  }
}

export class Tracer {
  private readonly roots: SpanNode[] = [];

  constructor(private readonly exporter?: Exporter) {}

  start(name: string, attributes: Attributes = {}): Span {
    const span = new SpanNode(name, Date.now(), attributes);
    this.roots.push(span);
    return span;
  }

  async flush(): Promise<number> {
    const finished = this.roots.splice(0).map((span) => span.finish());
    if (this.exporter && finished.length > 0) await this.exporter.export(finished);
    return finished.length;
  }

  async timed<T>(name: string, task: () => Promise<T>, attributes: Attributes = {}): Promise<T> {
    const span = this.start(name, attributes) as SpanNode;
    try {
      const value = await task();
      span.end({ status: 'ok' });
      return value;
    } catch (error) {
      span.end({ status: 'error', message: error instanceof Error ? error.message : String(error) });
      throw error;
    }
  }
}

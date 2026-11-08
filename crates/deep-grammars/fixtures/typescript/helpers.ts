export function chunk<T>(items: readonly T[], size: number): T[][] {
  if (size < 1) throw new RangeError('size must be at least 1');
  const out: T[][] = [];
  for (let index = 0; index < items.length; index += size) {
    out.push(items.slice(index, index + size));
  }
  return out;
}

export function groupBy<T, K extends PropertyKey>(items: readonly T[], key: (item: T) => K): Record<K, T[]> {
  const out = {} as Record<K, T[]>;
  for (const item of items) {
    const bucket = key(item);
    (out[bucket] ??= []).push(item);
  }
  return out;
}

export function sortBy<T>(items: readonly T[], compare: (a: T, b: T) => number): T[] {
  return [...items].sort(compare);
}

export function pick<T extends object, K extends keyof T>(value: T, keys: readonly K[]): Pick<T, K> {
  const out = {} as Pick<T, K>;
  for (const key of keys) out[key] = value[key];
  return out;
}

export function omit<T extends object, K extends keyof T>(value: T, keys: readonly K[]): Omit<T, K> {
  const out = { ...value };
  for (const key of keys) delete out[key];
  return out;
}

export function uniqueBy<T, K>(items: readonly T[], key: (item: T) => K): T[] {
  const seen = new Set<K>();
  const out: T[] = [];
  for (const item of items) {
    const marker = key(item);
    if (seen.has(marker)) continue;
    seen.add(marker);
    out.push(item);
  }
  return out;
}

export const range = (from: number, to = from, step = 1): number[] => {
  const start = to === from ? 0 : from;
  const out: number[] = [];
  for (let value = start; step > 0 ? value < to : value > to; value += step) out.push(value);
  return out;
};

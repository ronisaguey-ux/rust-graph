import { createHash } from 'node:crypto';
import * as os from 'node:os';

const VERSION = 2;

export const digest = (input, algorithm = 'sha256') =>
  createHash(algorithm).update(input).digest('hex');

export function memoize(fn, { key = (...args) => JSON.stringify(args) } = {}) {
  const store = new Map();
  return function memoized(...args) {
    const cacheKey = key(...args);
    if (store.has(cacheKey)) return store.get(cacheKey);
    const value = fn.apply(this, args);
    store.set(cacheKey, value);
    return value;
  };
}

export class TokenBucket {
  constructor({ rate, burst = rate }) {
    this.rate = rate;
    this.burst = burst;
    this.tokens = burst;
    this.last = Date.now();
  }

  take(cost = 1) {
    const now = Date.now();
    this.tokens = Math.min(this.burst, this.tokens + ((now - this.last) / 1000) * this.rate);
    this.last = now;
    if (this.tokens < cost) return false;
    this.tokens -= cost;
    return true;
  }
}

export default {
  version: VERSION,
  platform: `${os.platform()}-${os.arch()}`,
  cpus: os.cpus().length,
};

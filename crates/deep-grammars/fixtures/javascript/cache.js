export class Cache {
  #store = new Map();
  #limit;

  constructor(limit = 100) {
    if (limit <= 0) throw new RangeError('limit must be positive');
    this.#limit = limit;
  }

  get size() {
    return this.#store.size;
  }

  has(key) {
    return this.#store.has(key);
  }

  get(key) {
    const entry = this.#store.get(key);
    if (!entry) return undefined;
    const { value, expires } = entry;
    if (expires !== null && Date.now() > expires) {
      this.#store.delete(key);
      return undefined;
    }
    return value;
  }

  set(key, value, ttlMs = null) {
    if (this.#store.size >= this.#limit && !this.#store.has(key)) {
      const oldest = this.#store.keys().next().value;
      this.#store.delete(oldest);
    }
    this.#store.set(key, { value, expires: ttlMs === null ? null : Date.now() + ttlMs });
    return this;
  }

  async getOrLoad(key, loader, ttlMs) {
    const cached = this.get(key);
    if (cached !== undefined) return cached;
    const value = await loader(key);
    this.set(key, value, ttlMs);
    return value;
  }
}

export default new Cache(500);

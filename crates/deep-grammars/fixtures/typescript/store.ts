export type Listener<T> = (state: T, previous: T) => void;

export interface Store<T> {
  get(): T;
  set(patch: Partial<T> | ((current: T) => Partial<T>)): void;
  subscribe(listener: Listener<T>): () => void;
  select<K>(pick: (state: T) => K, listener: (value: K, previous: K) => void): () => void;
}

export function createStore<T extends object>(initial: T): Store<T> {
  let state = initial;
  const listeners = new Set<Listener<T>>();

  const notify = (previous: T) => {
    for (const listener of [...listeners]) listener(state, previous);
  };

  return {
    get: () => state,
    set(patch) {
      const previous = state;
      const changes = typeof patch === 'function' ? patch(state) : patch;
      state = { ...state, ...changes };
      notify(previous);
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    select<K>(pick: (state: T) => K, listener: (value: K, previous: K) => void) {
      let current = pick(state);
      return this.subscribe((next) => {
        const value = pick(next);
        if (Object.is(value, current)) return;
        const previous = current;
        current = value;
        listener(value, previous);
      });
    },
  };
}

export const keys = <T extends object>(value: T): (keyof T)[] => Object.keys(value) as (keyof T)[];

import type { CreateUser, Id, Page, Role, User } from './api_types';

export interface Repository<T, K> {
  find(id: K): Promise<T | null>;
  list(limit: number, cursor?: string): Promise<Page<T>>;
  create(input: T): Promise<K>;
  update(id: K, patch: Partial<T>): Promise<T>;
}

export class UserService {
  private readonly users = new Map<Id, User>();
  private sequence = 0;

  constructor(private readonly clock: () => Date = () => new Date()) {}

  async create(input: CreateUser): Promise<User> {
    this.sequence += 1;
    const user: User = {
      ...input,
      id: `u_${this.sequence}` as Id,
      createdAt: this.clock(),
    };
    this.users.set(user.id, user);
    return user;
  }

  async find(id: Id): Promise<User | null> {
    return this.users.get(id) ?? null;
  }

  async list(limit: number, cursor?: string): Promise<Page<User>> {
    const all = [...this.users.values()];
    const start = cursor ? all.findIndex((user) => user.id === cursor) + 1 : 0;
    const items = all.slice(start, start + limit);
    const next = start + limit < all.length ? items[items.length - 1]?.id ?? null : null;
    return { items, cursor: next, total: all.length };
  }

  async promote(id: Id, role: Role): Promise<User> {
    const user = await this.find(id);
    if (!user) throw new Error(`no user ${id}`);
    const updated: User = { ...user, role };
    this.users.set(id, updated);
    return updated;
  }
}

export function assertNever(value: never): never {
  throw new Error(`unexpected variant: ${JSON.stringify(value)}`);
}

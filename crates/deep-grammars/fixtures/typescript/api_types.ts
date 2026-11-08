export type Id = string & { readonly __brand: 'Id' };

export interface Page<T> {
  readonly items: T[];
  readonly cursor: string | null;
  readonly total?: number;
}

export interface User {
  id: Id;
  name: string;
  email: string;
  role: Role;
  createdAt: Date;
  address?: Address;
}

export type Role = 'admin' | 'editor' | 'viewer';

export interface Address {
  street: string;
  city: string;
  postalCode: string;
  country: string;
}

export type CreateUser = Omit<User, 'id' | 'createdAt'>;
export type UserPatch = Partial<Pick<User, 'name' | 'email' | 'address'>>;
export type RoleCounts = Record<Role, number>;

export function isPage<T>(value: unknown): value is Page<T> {
  return typeof value === 'object' && value !== null && Array.isArray((value as Page<T>).items);
}

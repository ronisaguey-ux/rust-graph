export enum Environment {
  Development = 'development',
  Staging = 'staging',
  Production = 'production',
}

export interface DatabaseConfig {
  host: string;
  port: number;
  name: string;
  ssl: boolean;
  pool: { min: number; max: number };
}

export interface ServerConfig {
  port: number;
  host: string;
  timeoutMs: number;
  cors: readonly string[];
}

export interface AppConfig {
  env: Environment;
  server: ServerConfig;
  database: DatabaseConfig;
  flags: Record<string, boolean>;
}

type DeepPartial<T> = { [K in keyof T]?: T[K] extends object ? DeepPartial<T[K]> : T[K] };

const defaults: AppConfig = {
  env: Environment.Development,
  server: { port: 8080, host: '127.0.0.1', timeoutMs: 30_000, cors: ['http://localhost:3000'] },
  database: { host: 'localhost', port: 5432, name: 'app', ssl: false, pool: { min: 1, max: 10 } },
  flags: {},
};

function merge<T>(base: T, patch: DeepPartial<T>): T {
  const result = { ...base } as T;
  for (const key of Object.keys(patch) as (keyof T)[]) {
    const value = patch[key];
    if (value === undefined) continue;
    const current = base[key];
    result[key] =
      typeof value === 'object' && value !== null && typeof current === 'object' && current !== null
        ? merge(current, value as DeepPartial<T[keyof T]>)
        : (value as T[keyof T]);
  }
  return result;
}

export function loadConfig(overrides: DeepPartial<AppConfig> = {}, env = process.env): AppConfig {
  const fromEnv: DeepPartial<AppConfig> = {
    env: (env.APP_ENV as Environment | undefined) ?? undefined,
    server: env.PORT ? { port: Number(env.PORT) } : undefined,
  };
  return merge(merge(defaults, fromEnv), overrides);
}

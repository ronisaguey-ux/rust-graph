export type Result<T, E = Error> = { ok: true; value: T } | { ok: false; error: E };

export const ok = <T>(value: T): Result<T, never> => ({ ok: true, value });
export const fail = <E>(error: E): Result<never, E> => ({ ok: false, error });

export function isOk<T, E>(result: Result<T, E>): result is { ok: true; value: T } {
  return result.ok;
}

export function unwrap<T, E>(result: Result<T, E>): T {
  if (result.ok) return result.value;
  throw result.error instanceof Error ? result.error : new Error(String(result.error));
}

export function map<T, U, E>(result: Result<T, E>, transform: (value: T) => U): Result<U, E> {
  return result.ok ? ok(transform(result.value)) : result;
}

export function flatMap<T, U, E>(result: Result<T, E>, transform: (value: T) => Result<U, E>): Result<U, E> {
  return result.ok ? transform(result.value) : result;
}

export function tryCatch<T>(task: () => T): Result<T> {
  try {
    return ok(task());
  } catch (error) {
    return fail(error instanceof Error ? error : new Error(String(error)));
  }
}

export async function tryAsync<T>(task: () => Promise<T>): Promise<Result<T>> {
  try {
    return ok(await task());
  } catch (error) {
    return fail(error instanceof Error ? error : new Error(String(error)));
  }
}

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

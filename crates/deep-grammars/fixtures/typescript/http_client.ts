export interface RequestOptions {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  headers?: Record<string, string>;
  body?: unknown;
  timeoutMs?: number;
  retries?: number;
}

export class HttpError extends Error {
  constructor(readonly status: number, readonly url: string, message?: string) {
    super(message ?? `HTTP ${status} for ${url}`);
    this.name = 'HttpError';
  }
}

const sleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

export class HttpClient {
  constructor(
    private readonly baseUrl: string,
    private readonly defaults: RequestOptions = {},
  ) {}

  private url(path: string): string {
    return new URL(path.replace(/^\//, ''), this.baseUrl.endsWith('/') ? this.baseUrl : `${this.baseUrl}/`).toString();
  }

  async request<T>(path: string, options: RequestOptions = {}): Promise<T> {
    const merged = { ...this.defaults, ...options };
    const attempt = async (): Promise<T> => {
      const controller = new AbortController();
      const timer = setTimeout(() => controller.abort(), merged.timeoutMs ?? 10_000);
      try {
        const response = await fetch(this.url(path), {
          method: merged.method ?? 'GET',
          headers: {
            accept: 'application/json',
            ...(merged.body === undefined ? {} : { 'content-type': 'application/json' }),
            ...merged.headers,
          },
          body: merged.body === undefined ? undefined : JSON.stringify(merged.body),
          signal: controller.signal,
        });
        if (!response.ok) throw new HttpError(response.status, this.url(path));
        return (await response.json()) as T;
      } finally {
        clearTimeout(timer);
      }
    };
    let lastError: unknown;
    const retries = merged.retries ?? 2;
    for (let index = 0; index <= retries; index += 1) {
      try {
        return await attempt();
      } catch (error) {
        lastError = error;
        if (error instanceof HttpError && error.status < 500) throw error;
        await sleep(200 * 2 ** index);
      }
    }
    throw lastError;
  }

  get<T>(path: string, options?: RequestOptions): Promise<T> {
    return this.request<T>(path, { ...options, method: 'GET' });
  }

  post<T>(path: string, body: unknown, options?: RequestOptions): Promise<T> {
    return this.request<T>(path, { ...options, method: 'POST', body });
  }
}

export type Handler<State> = (request: Request, state: State) => Promise<Response> | Response;

export interface Route<State> {
  readonly method: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  readonly path: string;
  readonly handler: Handler<State>;
}

export interface Match<State> {
  route: Route<State>;
  params: Record<string, string>;
}

const PARAM = /:([A-Za-z_][A-Za-z0-9_]*)/g;

function compile<State>(route: Route<State>): { pattern: RegExp; keys: string[] } {
  const keys: string[] = [];
  const source = route.path
    .replace(PARAM, (_match, key: string) => {
      keys.push(key);
      return '([^/]+)';
    })
    .replace(/\*/g, '.*');
  return { pattern: new RegExp(`^${source}$`), keys };
}

export class Router<State> {
  private readonly compiled: { route: Route<State>; pattern: RegExp; keys: string[] }[] = [];

  add(route: Route<State>): this {
    const { pattern, keys } = compile(route);
    this.compiled.push({ route, pattern, keys });
    return this;
  }

  get(path: string, handler: Handler<State>): this {
    return this.add({ method: 'GET', path, handler });
  }

  post(path: string, handler: Handler<State>): this {
    return this.add({ method: 'POST', path, handler });
  }

  match(method: string, url: string): Match<State> | null {
    for (const { route, pattern, keys } of this.compiled) {
      if (route.method !== method) continue;
      const found = pattern.exec(url);
      if (!found) continue;
      const params: Record<string, string> = {};
      keys.forEach((key, index) => {
        params[key] = decodeURIComponent(found[index + 1] ?? '');
      });
      return { route, params };
    }
    return null;
  }

  async handle(request: Request, state: State): Promise<Response> {
    const url = new URL(request.url);
    const match = this.match(request.method, url.pathname);
    if (!match) return new Response('not found', { status: 404 });
    return match.route.handler(request, state);
  }
}

export type Token =
  | { kind: 'number'; value: number }
  | { kind: 'name'; value: string }
  | { kind: 'op'; value: '+' | '-' | '*' | '/' | '(' | ')' };

export type Node =
  | { kind: 'literal'; value: number }
  | { kind: 'variable'; name: string }
  | { kind: 'unary'; operator: '-' | '+'; operand: Node }
  | { kind: 'binary'; operator: '+' | '-' | '*' | '/'; left: Node; right: Node };

export class ParseError extends Error {
  constructor(message: string, readonly position: number) {
    super(`${message} at ${position}`);
    this.name = 'ParseError';
  }
}

export function tokenize(source: string): Token[] {
  const tokens: Token[] = [];
  let at = 0;
  while (at < source.length) {
    const char = source[at];
    if (/\s/.test(char)) {
      at += 1;
      continue;
    }
    if (/[0-9]/.test(char)) {
      let end = at;
      while (end < source.length && /[0-9.]/.test(source[end])) end += 1;
      tokens.push({ kind: 'number', value: Number(source.slice(at, end)) });
      at = end;
      continue;
    }
    if (/[A-Za-z_]/.test(char)) {
      let end = at;
      while (end < source.length && /[A-Za-z0-9_]/.test(source[end])) end += 1;
      tokens.push({ kind: 'name', value: source.slice(at, end) });
      at = end;
      continue;
    }
    if ('+-*/()'.includes(char)) {
      tokens.push({ kind: 'op', value: char as '+' | '-' | '*' | '/' | '(' | ')' });
      at += 1;
      continue;
    }
    throw new ParseError(`unexpected ${char}`, at);
  }
  return tokens;
}

export function parse(source: string): Node {
  const tokens = tokenize(source);
  let at = 0;
  const peek = (): Token | undefined => tokens[at];

  const binary = (next: () => Node, operators: readonly string[]): Node => {
    let left = next();
    while (peek()?.kind === 'op' && operators.includes(peek()!.value)) {
      const operator = (tokens[at++] as { value: '+' | '-' | '*' | '/' }).value;
      left = { kind: 'binary', operator, left, right: next() };
    }
    return left;
  };

  const primary = (): Node => {
    const token = tokens[at++];
    if (!token) throw new ParseError('unexpected end', at);
    switch (token.kind) {
      case 'number':
        return { kind: 'literal', value: token.value };
      case 'name':
        return { kind: 'variable', name: token.value };
      case 'op':
        if (token.value === '(') {
          const inner = expression();
          if (peek()?.kind !== 'op' || (peek() as { value: string }).value !== ')') {
            throw new ParseError('expected )', at);
          }
          at += 1;
          return inner;
        }
        if (token.value === '-' || token.value === '+') {
          return { kind: 'unary', operator: token.value, operand: primary() };
        }
        throw new ParseError(`unexpected ${token.value}`, at);
    }
  };

  const product = (): Node => binary(primary, ['*', '/']);
  const sum = (): Node => binary(product, ['+', '-']);
  const expression = (): Node => sum();

  const root = expression();
  if (at !== tokens.length) throw new ParseError('trailing input', at);
  return root;
}

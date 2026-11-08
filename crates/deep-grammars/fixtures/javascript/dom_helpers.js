export function qs(selector, root = document) {
  const node = root.querySelector(selector);
  if (!node) throw new Error(`no element for ${selector}`);
  return node;
}

export function on(target, type, handler, options) {
  target.addEventListener(type, handler, options);
  return () => target.removeEventListener(type, handler, options);
}

export function debounce(fn, wait = 100) {
  let timer = null;
  return function debounced(...args) {
    if (timer !== null) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      fn.apply(this, args);
    }, wait);
  };
}

export function classNames(...values) {
  const out = [];
  for (const value of values) {
    if (!value) continue;
    if (typeof value === 'string') out.push(value);
    else if (Array.isArray(value)) out.push(classNames(...value));
    else for (const [name, on] of Object.entries(value)) if (on) out.push(name);
  }
  return out.filter(Boolean).join(' ');
}

export function render(template, data) {
  return template.replace(/\{\{\s*([\w.]+)\s*\}\}/g, (whole, path) =>
    path.split('.').reduce((value, key) => (value == null ? '' : value[key]), data) ?? ''
  );
}

export const ready = new Promise((resolve) => {
  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', resolve, { once: true });
  } else {
    resolve();
  }
});

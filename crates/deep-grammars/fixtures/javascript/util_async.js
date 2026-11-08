const TIMEOUT_MS = 5000;

function sleep(ms) {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function retry(task, attempts = 3, delay = 250) {
  let lastError;
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      return await task(attempt);
    } catch (error) {
      lastError = error;
      if (attempt < attempts) await sleep(delay * 2 ** (attempt - 1));
    }
  }
  throw lastError;
}

async function fetchJson(url, { headers = {}, method = 'GET', body } = {}) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  try {
    const response = await fetch(url, {
      method,
      headers: { accept: 'application/json', ...headers },
      body: body === undefined ? undefined : JSON.stringify(body),
      signal: controller.signal,
    });
    if (!response.ok) {
      throw new Error(`HTTP ${response.status} for ${url}`);
    }
    return await response.json();
  } finally {
    clearTimeout(timer);
  }
}

function withConcurrency(tasks, limit = 4) {
  const results = [];
  let index = 0;
  const workers = Array.from({ length: Math.min(limit, tasks.length) }, async () => {
    while (index < tasks.length) {
      const slot = index++;
      try {
        results[slot] = { value: await tasks[slot]() };
      } catch (error) {
        results[slot] = { error };
      }
    }
  });
  return Promise.all(workers).then(() => results);
}

module.exports = { retry, fetchJson, withConcurrency, sleep };

const LOG_LINE = /^(?<time>\d{2}:\d{2}:\d{2})\s+(?<level>INFO|WARN|ERROR)\s+(?<message>.*)$/;
const DURATION = /(\d+(?:\.\d+)?)(ms|s|m|h)/g;
const UNITS = { ms: 1, s: 1000, m: 60_000, h: 3_600_000 };

function parseLine(line) {
  const match = LOG_LINE.exec(line.trim());
  if (!match) return null;
  const { time, level, message } = match.groups;
  return { time, level, message, tags: message.match(/\[[^\]]+\]/g) || [] };
}

function toMillis(text) {
  let total = 0;
  for (const match of text.matchAll(DURATION)) {
    total += parseFloat(match[1]) * UNITS[match[2]];
  }
  return total;
}

function normalise(path) {
  return path
    .replace(/\/+/g, '/')
    .replace(/(^\/|\/$)/g, '')
    .split('/')
    .filter((part) => part !== '.' && part !== '')
    .reduce((parts, part) => {
      if (part === '..') parts.pop();
      else parts.push(part);
      return parts;
    }, [])
    .join('/');
}

module.exports = { parseLine, toMillis, normalise };

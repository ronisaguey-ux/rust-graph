'use strict';

const express = require('express');
const { readFile } = require('fs/promises');
const path = require('path');

const app = express();
app.use(express.json());

const items = new Map();
let nextId = 1;

function findItem(id) {
  const item = items.get(id);
  if (!item) {
    const error = new Error('item not found');
    error.status = 404;
    throw error;
  }
  return item;
}

app.get('/api/items', (req, res) => {
  const tag = req.query.tag;
  const values = [...items.values()].filter((item) => !tag || item.tags.includes(tag));
  res.json({ count: values.length, values });
});

app.post('/api/items', (req, res) => {
  const { name, tags = [] } = req.body || {};
  if (typeof name !== 'string' || !name.trim()) {
    return res.status(422).json({ error: 'name is required' });
  }
  const item = { id: nextId++, name: name.trim(), tags };
  items.set(item.id, item);
  return res.status(201).json(item);
});

app.get('/api/items/:id', async (req, res, next) => {
  try {
    const item = findItem(Number(req.params.id));
    res.json(item);
  } catch (error) {
    next(error);
  }
});

app.get('/health', async (req, res) => {
  const packageJson = JSON.parse(await readFile(path.join(__dirname, 'package.json'), 'utf8'));
  res.json({ ok: true, version: packageJson.version });
});

app.use((error, req, res, next) => {
  res.status(error.status || 500).json({ error: error.message });
});

module.exports = app;

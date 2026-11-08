const { EventEmitter } = require('events');

class Queue extends EventEmitter {
  constructor({ highWater = 16 } = {}) {
    super();
    this.items = [];
    this.highWater = highWater;
    this.paused = false;
  }

  push(item) {
    this.items.push(item);
    this.emit('push', item, this.items.length);
    if (this.items.length >= this.highWater && !this.paused) {
      this.paused = true;
      this.emit('pause');
    }
    return this.items.length;
  }

  shift() {
    if (this.items.length === 0) {
      this.emit('drain');
      return undefined;
    }
    const item = this.items.shift();
    if (this.paused && this.items.length < this.highWater / 2) {
      this.paused = false;
      this.emit('resume');
    }
    return item;
  }

  onDrain(handler) {
    this.on('drain', handler);
    return () => this.off('drain', handler);
  }
}

function pipe(source, sinks) {
  const queue = new Queue();
  source.on('data', (chunk) => {
    for (const sink of sinks) sink.write(chunk);
    queue.push(chunk.length);
  });
  return new Promise((resolve, reject) => {
    source.on('end', () => resolve(queue.items.length));
    source.on('error', reject);
  });
}

module.exports = { Queue, pipe };

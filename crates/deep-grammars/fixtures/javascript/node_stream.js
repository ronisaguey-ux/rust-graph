const { Transform, pipeline } = require('stream');
const { createGzip } = require('zlib');
const fs = require('fs');

class CsvRows extends Transform {
  constructor(options = {}) {
    super({ ...options, objectMode: false });
    this.buffer = '';
    this.header = null;
    this.count = 0;
  }

  _transform(chunk, encoding, done) {
    this.buffer += chunk.toString('utf8');
    let newline;
    while ((newline = this.buffer.indexOf('\n')) !== -1) {
      const line = this.buffer.slice(0, newline).trim();
      this.buffer = this.buffer.slice(newline + 1);
      if (!line) continue;
      const cells = line.split(',').map((cell) => cell.replace(/^"|"$/g, ''));
      if (this.header === null) {
        this.header = cells;
        continue;
      }
      const row = Object.fromEntries(this.header.map((name, index) => [name, cells[index] ?? '']));
      this.count += 1;
      this.push(`${JSON.stringify(row)}\n`);
    }
    done();
  }

  _flush(done) {
    if (this.buffer.trim()) this.push(this.buffer);
    this.emit('summary', { rows: this.count });
    done();
  }
}

function convert(input, output) {
  return new Promise((resolve, reject) => {
    const rows = new CsvRows();
    rows.on('summary', (stats) => console.log('rows', stats.rows));
    pipeline(fs.createReadStream(input), rows, createGzip(), fs.createWriteStream(output), (error) =>
      error ? reject(error) : resolve()
    );
  });
}

if (require.main === module) {
  convert(process.argv[2], process.argv[3]).catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}

module.exports = { CsvRows, convert };

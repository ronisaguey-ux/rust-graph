CREATE TABLE IF NOT EXISTS repositories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    owner_id INTEGER NOT NULL REFERENCES accounts (id) ON DELETE CASCADE ON UPDATE CASCADE,
    name VARCHAR(120) NOT NULL,
    slug TEXT UNIQUE,
    score NUMERIC(10, 2) CHECK (score >= 0),
    ratio DOUBLE PRECISION DEFAULT 0.0,
    tags TEXT[],
    metadata JSON,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT NOW(),
    CONSTRAINT repositories_name_key UNIQUE (owner_id, name),
    FOREIGN KEY (owner_id) REFERENCES accounts (id) ON DELETE RESTRICT,
    CHECK (length(name) > 0)
);

CREATE INDEX IF NOT EXISTS repositories_owner_index ON repositories (owner_id);

ALTER TABLE repositories
    ADD COLUMN archived BOOLEAN NOT NULL DEFAULT FALSE,
    ADD CONSTRAINT repositories_slug_key UNIQUE (slug);

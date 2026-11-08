-- The schema of the scanner's own store.
CREATE TABLE repositories (
    id INTEGER PRIMARY KEY,
    name VARCHAR(255) NOT NULL UNIQUE,
    owner_id INTEGER REFERENCES accounts (id),
    default_branch TEXT DEFAULT 'main',
    created_at TIMESTAMP WITH TIME ZONE,
    score NUMERIC(10, 2) CHECK (score >= 0)
);

CREATE TABLE accounts (
    id INTEGER PRIMARY KEY,
    login VARCHAR(64) NOT NULL,
    email TEXT
);

CREATE INDEX IF NOT EXISTS repositories_owner ON repositories (owner_id);

ALTER TABLE repositories ADD COLUMN archived BOOLEAN NOT NULL DEFAULT false;

CREATE VIEW active_repositories AS
    SELECT r.id, r.name, a.login
    FROM repositories AS r
    JOIN accounts AS a ON a.id = r.owner_id
    WHERE r.archived = false;

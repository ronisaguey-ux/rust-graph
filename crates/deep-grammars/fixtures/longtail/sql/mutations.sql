INSERT INTO accounts (login, email)
VALUES
    ('roni', 'roni@example.com'),
    ('tomer', 'tomer@example.com');

INSERT INTO repositories (name, owner_id)
SELECT 'deep-scan', id FROM accounts WHERE login = 'roni';

UPDATE repositories
SET name = 'rust-graph', score = score + 1
WHERE name = 'deep-scan' AND archived = false;

DELETE FROM events
WHERE created_at < '2025-01-01' AND repo_id IN (SELECT id FROM repositories WHERE archived = true);

SELECT DISTINCT owner_id, CAST(score AS NUMERIC(12, 3)) AS rounded
FROM repositories
WHERE score BETWEEN 1 AND 9.5
UNION
SELECT owner_id, 0 FROM accounts
WHERE login IS NOT NULL;

-- Broken on purpose: the column list is not closed and VALUES is misspelled.
SELECT id, name
FROM repositories
WHERE owner_id = ;

INSERT INTO accounts (login, email
VALUE ('x', 'x@example.com');

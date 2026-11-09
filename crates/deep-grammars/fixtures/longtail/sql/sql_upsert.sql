INSERT INTO accounts (login, email, created_at)
VALUES ('octo', 'octo@example.com', NOW())
ON CONFLICT (login) DO UPDATE
    SET email = EXCLUDED.email,
        updated_at = NOW()
    WHERE accounts.email <> EXCLUDED.email
RETURNING id, login AS name, email;

UPDATE accounts
   SET email = lower(email),
       active = TRUE
  FROM repositories r
 WHERE r.owner_id = accounts.id
   AND r.archived = FALSE
RETURNING accounts.id, accounts.email;

DELETE FROM sessions
 USING accounts
 WHERE sessions.account_id = accounts.id
   AND accounts.active = FALSE
RETURNING *;

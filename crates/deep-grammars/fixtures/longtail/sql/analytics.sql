WITH recent AS (
    SELECT repo_id, COUNT(*) AS n, MAX(created_at) AS last_seen
    FROM events
    WHERE created_at >= '2026-01-01'
    GROUP BY repo_id
    HAVING COUNT(*) > 5
)
SELECT r.name,
       a.login AS owner,
       recent.n,
       recent.n / 7 AS per_day,
       CASE WHEN recent.n > 100 THEN 'hot' ELSE 'steady' END AS band
FROM recent
INNER JOIN repositories r ON r.id = recent.repo_id
LEFT OUTER JOIN accounts a ON a.id = r.owner_id
WHERE r.name LIKE 'graph-%'
  AND a.login NOT IN ('bot', 'ci')
  AND EXISTS (SELECT 1 FROM stars s WHERE s.repo_id = r.id)
ORDER BY recent.n DESC
LIMIT 25 OFFSET 0;

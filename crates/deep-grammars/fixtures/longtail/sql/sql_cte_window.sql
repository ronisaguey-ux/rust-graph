-- Reporting query over the events and repositories tables.
/* Both comment styles are lexed as trivia:
   a block comment may hold a quote and a semicolon; it is not code. */
WITH recent AS (
    SELECT repo_id,
           COUNT(*) AS n,
           MAX(created_at) AS last_seen
    FROM events
    WHERE created_at >= '2026-01-01'
    GROUP BY repo_id
    HAVING COUNT(*) > 5
), ranked AS (
    SELECT repo_id,
           n,
           RANK() OVER (PARTITION BY repo_id ORDER BY n DESC) AS rnk,
           SUM(n) OVER (PARTITION BY repo_id ORDER BY n
                        ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS running,
           AVG(n) FILTER (WHERE n > 10) OVER w AS big_avg
    FROM recent
    WINDOW w AS (PARTITION BY repo_id)
)
SELECT r.name,
       a.login AS owner,
       ranked.n,
       ranked.rnk,
       ranked.running,
       CASE
           WHEN ranked.n > 100 THEN 'hot'
           WHEN ranked.n > 10 THEN 'warm'
           ELSE 'steady'
       END AS band,
       'it''s quoted' AS note
FROM ranked
INNER JOIN repositories r ON r.id = ranked.repo_id
LEFT OUTER JOIN accounts a ON a.id = r.owner_id
WHERE r.name LIKE 'graph-%'
  AND a.login NOT IN ('bot', 'ci')
  AND r.archived IS NOT NULL
  AND r.retired BETWEEN 1 AND 3
  AND EXISTS (SELECT 1 FROM stars s WHERE s.repo_id = r.id)
ORDER BY ranked.n DESC NULLS LAST
LIMIT 25 OFFSET 0;

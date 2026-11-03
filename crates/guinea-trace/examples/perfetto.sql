-- trace_processor query -f crates/guinea-trace/examples/perfetto.sql output.pftrace
-- Wall intervals and Guinea's observation-adjusted elapsed time are different
-- measurements. Neither measures CPU execution time.
SELECT
  name AS operation,
  EXTRACT_ARG(arg_set_id, 'debug.actor') AS actor,
  EXTRACT_ARG(arg_set_id, 'debug.message') AS message,
  COUNT(*) AS count,
  AVG(dur) / 1000000.0 AS mean_wall_ms,
  MAX(dur) / 1000000.0 AS max_wall_ms,
  AVG(EXTRACT_ARG(arg_set_id, 'debug.guinea_took_ns')) / 1000000.0 AS mean_adjusted_ms
FROM slice
WHERE name IN ('action', 'handle', 'span') AND dur >= 0
GROUP BY operation, actor, message;

SELECT t.name, MIN(c.value) AS min_value, MAX(c.value) AS max_value
FROM counter c JOIN counter_track t ON c.track_id = t.id
GROUP BY t.name;

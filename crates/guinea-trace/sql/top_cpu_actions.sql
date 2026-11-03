-- One sample belongs to its nearest Guinea action in the causal chain.
-- Background work is included; wall time and waiting are not CPU time.
WITH RECURSIVE ancestry(sample_id, callsite_id, depth) AS (
  SELECT id, callsite_id, 0 FROM perf_sample WHERE callsite_id IS NOT NULL
  UNION ALL
  SELECT a.sample_id, c.parent_id, a.depth + 1
  FROM ancestry a JOIN stack_profile_callsite c ON c.id = a.callsite_id
  WHERE c.parent_id IS NOT NULL
), matches AS (
  SELECT a.sample_id, SUBSTR(f.name, LENGTH('[Guinea] action ') + 1) AS action,
    ROW_NUMBER() OVER (PARTITION BY a.sample_id ORDER BY a.depth) AS nearest
  FROM ancestry a JOIN stack_profile_callsite c ON c.id = a.callsite_id
  JOIN stack_profile_frame f ON f.id = c.frame_id
  WHERE f.name GLOB '[[]Guinea] action *'
), counts AS (
  SELECT action, COUNT(*) AS samples FROM matches WHERE nearest = 1 GROUP BY action
), invocations AS (
  SELECT EXTRACT_ARG(arg_set_id, 'debug.message') AS action, COUNT(*) AS invocations
  FROM slice WHERE COALESCE(EXTRACT_ARG(arg_set_id, 'debug.guinea_kind'), name) = 'action' GROUP BY action
), sampling AS (
  SELECT CAST(str_value AS REAL) / 1e6 AS interval_ms
  FROM metadata WHERE name = 'trace_attribute.rs.guinea.cpu.interval_ns'
)
SELECT action, samples,
  samples * (SELECT interval_ms FROM sampling) AS estimated_cpu_ms,
  100.0 * samples / (SELECT COUNT(*) FROM perf_sample) AS process_cpu_percent,
  invocations,
  samples * (SELECT interval_ms FROM sampling) / NULLIF(invocations, 0) AS estimated_cpu_ms_per_invocation
FROM counts LEFT JOIN invocations USING (action)
ORDER BY samples DESC LIMIT 20;

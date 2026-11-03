-- Validate the actual ingestion, including async causal edges and clock health.
SELECT
  (SELECT COUNT(*) FROM slice WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_kind') IN ('action', 'handle') AND dur >= 0) = 2 AS intervals,
  (SELECT COUNT(*) FROM slice WHERE name = 'push · Count' AND dur = 0) = 1 AS marks,
  (SELECT COUNT(*) FROM flow) = 2 AS flows,
  (SELECT COUNT(*) FROM counter WHERE value = 4096) = 1 AS counters,
  (SELECT COUNT(*) FROM slice
       WHERE name = 'Refresh → Worker' AND EXTRACT_ARG(arg_set_id, 'debug.actor') = 'Worker') = 1 AS actors,
  (SELECT COUNT(*) FROM slice
       WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_kind') IN ('action', 'handle')
         AND EXTRACT_ARG(arg_set_id, 'debug.guinea_took_ns') >= 0) = 2 AS durations,
  (SELECT COUNT(*) FROM metadata m
       WHERE m.name GLOB 'trace_attribute.rs.guinea.thread.*'
         AND JSON_EXTRACT(m.str_value, '$.name') = 'profile-worker'
         AND EXISTS (SELECT 1 FROM thread t
           WHERE t.tid = JSON_EXTRACT(m.str_value, '$.tid')
             AND INSTR(t.name, JSON_EXTRACT(m.str_value, '$.rust_id')) > 0)) = 1 AS threads,
  NOT EXISTS (
    SELECT 1 FROM stats WHERE severity IN ('error', 'data_loss') AND value != 0
  ) AS healthy;

INCLUDE PERFETTO MODULE guinea.memory;
CREATE PERFETTO VIEW window_stats AS
SELECT * FROM guinea_scope_memory_growth(trace_start(),
  (SELECT c.ts FROM counter c JOIN counter_track t ON c.track_id = t.id
   WHERE t.name = 'test.window.end' LIMIT 1));
SELECT
  (SELECT COUNT(*) FROM window_stats WHERE label = 'Growth'
    AND growth_bytes = 1024 AND growth_bytes_per_second > 0
    AND allocated_in_window_bytes = 1024 AND freed_in_window_bytes = 0) = 1
  AND (SELECT COUNT(*) FROM window_stats WHERE label = 'Churn'
    AND growth_bytes = 0 AND churn_bytes = 16384 AND live_bytes = 0) = 1
  AND (SELECT COUNT(*) FROM window_stats WHERE label = 'Held'
    AND closed = 1 AND retained_after_close_bytes = 128 AND retained_after_close_ns >= 0) = 1
  AND (SELECT COUNT(*) FROM window_stats WHERE label = 'Cache'
    AND growth_bytes = 0 AND live_bytes = 64 AND retained_after_close_bytes IS NULL) = 1
  AND (SELECT COUNT(*) FROM guinea_scope_memory_stats WHERE scope_id = 99
    AND label = 'Application' AND samples = 1 AND growth_bytes IS NULL AND growth_bytes_per_second IS NULL) = 1
  AND (SELECT COUNT(*) FROM guinea_scope_memory_stats WHERE retained_after_close_bytes > 0) = 0
  AND (SELECT COUNT(*) FROM window_stats WHERE label IS NOT NULL AND file IS NOT NULL AND line > 0 AND col > 0) = 4
  AND NOT EXISTS (SELECT 1 FROM track WHERE name = 'Scope memory samples')
  AND NOT EXISTS (SELECT 1 FROM slice WHERE name GLOB 'guinea.memory.*')
  AND NOT EXISTS (SELECT 1 FROM stats WHERE severity IN ('error','data_loss') AND value != 0)
  AS valid;

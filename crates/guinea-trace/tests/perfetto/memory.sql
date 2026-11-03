SELECT
  (SELECT COUNT(*) FROM slice WHERE EXTRACT_ARG(arg_set_id, 'debug.guinea_kind') = 'action'
     AND EXTRACT_ARG(arg_set_id, 'debug.memory_allocated_bytes') = 1536
     AND EXTRACT_ARG(arg_set_id, 'debug.memory_live_bytes_at_end') = 1536
     AND EXTRACT_ARG(arg_set_id, 'debug.memory_peak_bytes') = 1536) = 1
  AND (SELECT MAX(c.value) FROM counter c JOIN counter_track t ON c.track_id = t.id
       WHERE t.name = 'Rust heap / live bytes') >= 1536
  AND (SELECT MAX(c.value) FROM counter c JOIN counter_track t ON c.track_id = t.id
       WHERE t.name = 'Guinea scopes / live bytes') = 1024
  AND (SELECT MIN(c.value) FROM counter c JOIN counter_track t ON c.track_id = t.id
       WHERE t.name = 'Guinea scopes / live bytes') = 0
  AND (SELECT COUNT(*) FROM counter_track) = 3
  AND NOT EXISTS (SELECT 1 FROM track WHERE name = 'Scope memory samples')
  AND NOT EXISTS (SELECT 1 FROM slice WHERE name GLOB 'guinea.memory.*')
  AND (SELECT COUNT(*) FROM metadata
       WHERE name GLOB 'trace_attribute.rs.guinea.memory.sample.*') = 2
  AND NOT EXISTS (
    SELECT 1 FROM stats WHERE severity IN ('error', 'data_loss') AND value != 0
  ) AS valid;

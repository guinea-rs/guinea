-- Service samples are native trace attributes, never visible timeline events.
-- JSON stores the original timestamp because trace attributes are untimed.
CREATE PERFETTO VIEW guinea_scope_memory_samples AS
SELECT ROW_NUMBER() OVER (ORDER BY JSON_EXTRACT(m.str_value, '$.ts'),
    JSON_EXTRACT(m.str_value, '$.batch'), CAST(s.key AS INT)) AS id,
  JSON_EXTRACT(m.str_value, '$.ts') AS ts,
  JSON_EXTRACT(s.value, '$.group_id') AS group_id,
  JSON_EXTRACT(s.value, '$.scope_id') AS scope_id,
  JSON_EXTRACT(s.value, '$.parent_group_id') AS parent_group_id,
  JSON_EXTRACT(s.value, '$.closed') AS closed,
  JSON_EXTRACT(s.value, '$.closed_at_ns') AS closed_at,
  JSON_EXTRACT(s.value, '$.allocated_bytes') AS allocated_bytes,
  JSON_EXTRACT(s.value, '$.freed_bytes') AS freed_bytes,
  JSON_EXTRACT(s.value, '$.live_bytes') AS live_bytes,
  JSON_EXTRACT(s.value, '$.peak_bytes') AS peak_bytes
FROM metadata m, JSON_EACH(m.str_value, '$.scopes') s
WHERE m.name GLOB 'trace_attribute.rs.guinea.memory.sample.*'
  AND JSON_EXTRACT(m.str_value, '$.schema_version') = 1;

CREATE PERFETTO VIEW guinea_scope_memory_declarations AS
SELECT
  JSON_EXTRACT(str_value, '$.group_id') AS group_id,
  JSON_EXTRACT(str_value, '$.kind') AS kind,
  JSON_EXTRACT(str_value, '$.name') AS name,
  JSON_EXTRACT(str_value, '$.file') AS file,
  JSON_EXTRACT(str_value, '$.line') AS line,
  JSON_EXTRACT(str_value, '$.column') AS col,
  JSON_EXTRACT(str_value, '$.crate_dir') AS crate_dir
FROM metadata WHERE name GLOB 'trace_attribute.rs.guinea.memory.declaration.*';
-- Endpoints are samples inside the requested window. A single sample cannot
-- establish growth or a rate. Large allocation/free turnover is separate from
-- retained growth. A closed origin retaining memory is a candidate, not proof
-- of a leak: another live scope may legitimately own the object.
CREATE PERFETTO FUNCTION guinea_scope_memory_growth(start_ts TIMESTAMP, end_ts TIMESTAMP)
RETURNS TABLE(
  group_id LONG, scope_id LONG, label STRING, file STRING, line LONG, col LONG,
  samples LONG, first_ts TIMESTAMP, last_ts TIMESTAMP,
  live_bytes LONG, growth_bytes LONG, growth_bytes_per_second DOUBLE,
  allocated_in_window_bytes LONG, freed_in_window_bytes LONG, churn_bytes LONG,
  closed LONG, retained_after_close_bytes LONG, retained_after_close_ns DURATION
) AS
WITH ranked AS (
  SELECT *,
    ROW_NUMBER() OVER (PARTITION BY group_id ORDER BY ts, id) AS first_n,
    ROW_NUMBER() OVER (PARTITION BY group_id ORDER BY ts DESC, id DESC) AS last_n,
    COUNT(*) OVER (PARTITION BY group_id) AS samples
  FROM guinea_scope_memory_samples WHERE ts BETWEEN $start_ts AND $end_ts
), labels AS (
  SELECT group_id,
    COALESCE(MAX(CASE WHEN kind = 'segment' THEN name END),
      GROUP_CONCAT(CASE WHEN kind = 'feature' THEN name END, ', '),
      GROUP_CONCAT(CASE WHEN kind = 'plugin' THEN name END, ', '),
      MAX(CASE WHEN kind = 'role' THEN name END)) AS label
  FROM guinea_scope_memory_declarations GROUP BY group_id
), sites AS (
  SELECT *, ROW_NUMBER() OVER (PARTITION BY group_id ORDER BY kind = 'segment' DESC, kind = 'role', name) AS n
  FROM guinea_scope_memory_declarations
)
SELECT last.group_id, last.scope_id, labels.label, sites.file, sites.line, sites.col,
  last.samples, first.ts AS first_ts, last.ts AS last_ts, last.live_bytes,
  CASE WHEN last.samples > 1 THEN last.live_bytes - first.live_bytes END AS growth_bytes,
  CASE WHEN last.ts > first.ts
    THEN (last.live_bytes - first.live_bytes) * 1e9 / (last.ts - first.ts) END AS growth_bytes_per_second,
  CASE WHEN last.samples > 1 THEN last.allocated_bytes - first.allocated_bytes END AS allocated_in_window_bytes,
  CASE WHEN last.samples > 1 THEN last.freed_bytes - first.freed_bytes END AS freed_in_window_bytes,
  CASE WHEN last.samples > 1 THEN last.allocated_bytes - first.allocated_bytes
    + last.freed_bytes - first.freed_bytes END AS churn_bytes,
  last.closed,
  CASE WHEN last.closed THEN last.live_bytes END AS retained_after_close_bytes,
  CASE WHEN last.closed AND last.live_bytes > 0 THEN MAX(0, last.ts - last.closed_at) END AS retained_after_close_ns
FROM ranked first JOIN ranked last USING (group_id)
LEFT JOIN labels USING (group_id)
LEFT JOIN sites ON sites.group_id = last.group_id AND sites.n = 1
WHERE first.first_n = 1 AND last.last_n = 1;

CREATE PERFETTO VIEW guinea_scope_memory_stats AS
SELECT * FROM guinea_scope_memory_growth(trace_start(), trace_end());

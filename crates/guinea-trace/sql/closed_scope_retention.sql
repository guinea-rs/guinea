INCLUDE PERFETTO MODULE guinea.memory;
SELECT label, group_id, scope_id, file, line,
  retained_after_close_bytes, retained_after_close_ns / 1e6 AS ms_after_close
FROM guinea_scope_memory_stats WHERE retained_after_close_bytes > 0
ORDER BY retained_after_close_bytes DESC LIMIT 30;

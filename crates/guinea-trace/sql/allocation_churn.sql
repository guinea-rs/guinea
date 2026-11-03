INCLUDE PERFETTO MODULE guinea.memory;
SELECT label, group_id, scope_id, file, line,
  allocated_in_window_bytes, freed_in_window_bytes, growth_bytes, churn_bytes
FROM guinea_scope_memory_stats WHERE churn_bytes > 0
ORDER BY churn_bytes DESC LIMIT 30;

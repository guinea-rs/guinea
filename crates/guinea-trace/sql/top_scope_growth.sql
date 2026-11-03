INCLUDE PERFETTO MODULE guinea.memory;
SELECT label, group_id, scope_id, file, line, growth_bytes,
  growth_bytes_per_second, live_bytes, samples
FROM guinea_scope_memory_stats WHERE growth_bytes > 0
ORDER BY growth_bytes DESC, growth_bytes_per_second DESC LIMIT 30;

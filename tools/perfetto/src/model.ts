export const metricIds = [
  'hot_actions',
  'memory',
  'peaks',
  'retention',
  'scopes',
  'growth',
  'churn',
  'events',
] as const;
export type MetricId = (typeof metricIds)[number];
export function parseMetrics(value: unknown): MetricId[] {
  if (
    !Array.isArray(value) ||
    !value.every((metric) => metricIds.includes(metric as MetricId))
  ) {
    throw new Error('Unsupported Guinea profiling metrics');
  }
  return [...new Set(value as MetricId[])];
}

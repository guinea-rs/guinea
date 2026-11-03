import type {DashboardItem} from '../dev.perfetto.DataExplorer/dashboard/dashboard_registry';
import type {Dataset} from './session';
import type {MetricId} from './model';
import {queries} from './sql';

export const datasets = Object.fromEntries(
  Object.entries(queries).map(([id, query]) => [id, {id, ...query}]),
) as Record<MetricId, Dataset>;

export interface Section {
  id: string;
  title: string;
  items: DashboardItem[];
}
const bar = (
  id: MetricId,
  title: string,
  column: string,
  measure: string,
  row: number,
  col = 0,
  width = 24,
): DashboardItem => ({
  kind: 'chart',
  sourceNodeId: id,
  col,
  row,
  colSpan: width,
  rowSpan: 6,
  config: {
    id: `${id}-${measure}`,
    name: title,
    chartType: 'bar',
    column,
    measureColumn: measure,
    aggregation: 'SUM',
    orientation: 'horizontal',
  },
});
const line = (
  id: MetricId,
  title: string,
  group: string,
  row: number,
): DashboardItem => ({
  kind: 'chart',
  sourceNodeId: id,
  col: 0,
  row,
  colSpan: 24,
  rowSpan: 7,
  config: {
    id,
    name: title,
    chartType: 'line',
    column: 'time_ms',
    yColumn: 'memory_mib',
    groupColumn: group,
  },
});

export function sectionsFor(metrics: readonly MetricId[]): Section[] {
  const sections: Section[] = [
    {
      id: 'cpu',
      title: 'CPU',
      items: [
        bar(
          'hot_actions',
          'Hot Guinea actions · estimated CPU, ms',
          'action',
          'estimated_cpu_ms',
          0,
        ),
        bar(
          'hot_actions',
          'Process CPU share · %',
          'action',
          'process_cpu_percent',
          6,
          0,
          12,
        ),
        bar(
          'hot_actions',
          'Estimated CPU per invocation · ms',
          'action',
          'estimated_cpu_ms_per_invocation',
          6,
          12,
          12,
        ),
      ],
    },
    {
      id: 'memory',
      title: 'Memory',
      items: [
        line('memory', 'Process and Rust · MiB', 'series', 0),
        bar('peaks', 'Peak memory · MiB', 'series', 'peak_mib', 7),
        bar(
          'retention',
          'Retained after scope close · MiB',
          'scope',
          'retained_mib',
          13,
        ),
      ],
    },
    {
      id: 'scopes',
      title: 'Scopes',
      items: [
        line('scopes', 'Scope memory · MiB', 'scope', 0),
        bar('growth', 'Growth · MiB', 'scope', 'growth_mib', 7, 0, 12),
        bar(
          'churn',
          'Allocated + freed · MiB',
          'scope',
          'turnover_mib',
          7,
          12,
          12,
        ),
      ],
    },
    {id: 'events', title: 'Events', items: []},
  ];
  return sections
    .map((section) => ({
      ...section,
      items: section.items.filter(
        (item) =>
          item.kind !== 'chart' ||
          metrics.includes(item.sourceNodeId as MetricId),
      ),
    }))
    .filter(
      (section) =>
        section.items.length > 0 ||
        (section.id === 'events' && metrics.includes('events')),
    );
}

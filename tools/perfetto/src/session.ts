import type {MetricId} from './model.js';

export interface QueryResult {
  columns(): string[];
  error(): string | undefined;
}
export interface QueryEngine {
  query(sql: string): Promise<QueryResult>;
}
export interface Dataset {
  id: MetricId;
  sql: string;
  schemaSql: string;
}
export interface PreparedDataset {
  id: MetricId;
  tableName: string;
  columns: string[];
}

// Finish all SQL before handing sources to Perfetto's Dashboard component.
// Errors remain errors; a failed source is never published as an empty chart.
export async function prepareDatasets(
  engine: QueryEngine,
  datasets: readonly Dataset[],
  memoryModule: string,
): Promise<PreparedDataset[]> {
  const run = async (sql: string) => {
    const result = await engine.query(sql);
    if (result.error()) throw new Error(result.error());
    return result;
  };
  if (
    datasets.some((dataset) =>
      ['retention', 'scopes', 'growth', 'churn'].includes(dataset.id),
    )
  ) {
    await run(memoryModule);
  }
  const prepared: PreparedDataset[] = [];
  for (const dataset of datasets) {
    if (!/^[a-z_]+$/.test(dataset.id))
      throw new Error('Invalid metric identifier');
    const tableName = `guinea_profile_${dataset.id}`;
    await run(dataset.sql);
    const result = await run(dataset.schemaSql);
    prepared.push({id: dataset.id, tableName, columns: result.columns()});
  }
  return prepared;
}

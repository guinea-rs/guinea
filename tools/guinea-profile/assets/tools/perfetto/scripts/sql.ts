import {readFileSync} from 'node:fs';
import {createRequire} from 'node:module';
import {dirname, resolve, join} from 'node:path';
import {fileURLToPath} from 'node:url';
import {metricIds, type MetricId} from '../src/model.ts';

export interface RenderedQuery {
  sql: string;
  schemaSql: string;
}

export function renderStateQueries(dependencies = join(root, 'package.json')) {
  const nunjucks = createRequire(dependencies)(
    'nunjucks',
  ) as typeof import('nunjucks');
  const templates = new nunjucks.Environment(
    new nunjucks.FileSystemLoader(join(root, 'sql')),
    {
      autoescape: false,
      throwOnUndefined: true,
    },
  );
  return {
    presence: templates.render('state-presence.sql.njk'),
    module: templates.render('state-module.sql.njk'),
    selection: templates.render('state-selection.sql.njk', {
      event_id: '__EVENT_ID__',
    }),
  };
}

export function renderEventRanking(
  dependencies = join(root, 'package.json'),
): string {
  const nunjucks = createRequire(dependencies)(
    'nunjucks',
  ) as typeof import('nunjucks');
  return new nunjucks.Environment(
    new nunjucks.FileSystemLoader(join(root, 'sql')),
    {
      autoescape: false,
      throwOnUndefined: true,
    },
  ).render('event-ranking.sql.njk');
}

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
export function renderQueries(
  dependencies = join(root, 'package.json'),
): Record<MetricId, RenderedQuery> {
  const nunjucks = createRequire(dependencies)(
    'nunjucks',
  ) as typeof import('nunjucks');
  const templates = new nunjucks.Environment(
    new nunjucks.FileSystemLoader(join(root, 'sql')),
    {
      autoescape: false,
      throwOnUndefined: true,
    },
  );
  const queries = {} as Record<MetricId, RenderedQuery>;
  for (const metric of metricIds) {
    const query =
      metric === 'hot_actions'
        ? readFileSync(
            join(root, '../../crates/guinea-trace/sql/top_cpu_actions.sql'),
            'utf8',
          )
        : templates.render(`${metric}.sql.njk`);
    queries[metric] = {
      sql: templates.render('materialize.sql.njk', {metric, query}),
      schemaSql: templates.render('schema.sql.njk', {metric}),
    };
  }
  return queries;
}

export function renderCapabilities(
  dependencies = join(root, 'package.json'),
): string {
  const nunjucks = createRequire(dependencies)(
    'nunjucks',
  ) as typeof import('nunjucks');
  const templates = new nunjucks.Environment(
    new nunjucks.FileSystemLoader(join(root, 'sql')),
    {autoescape: false, throwOnUndefined: true},
  );
  return templates.render('capabilities.sql.njk');
}

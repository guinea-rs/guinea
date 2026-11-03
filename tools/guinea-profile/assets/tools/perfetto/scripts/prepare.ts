// Build adapter: install the typed plugin into an isolated, pinned Perfetto tree.
import {cpSync, mkdirSync, readFileSync, writeFileSync} from 'node:fs';
import {dirname, join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {
  renderQueries,
  renderCapabilities,
  renderEventRanking,
  renderStateQueries,
} from './sql.ts';

const source = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const repository = resolve(source, '../..');
const checkout = resolve(process.argv[2]);
const destination = join(checkout, 'ui/src/plugins/rs.guinea.Profile');
mkdirSync(destination, {recursive: true});
cpSync(join(source, 'src'), destination, {recursive: true});
const sql = (name: string) =>
  readFileSync(join(repository, 'crates/guinea-trace/sql', name), 'utf8');
writeFileSync(
  join(destination, 'state-sql.ts'),
  `// Generated from SQL templates.\nexport const stateSql = ${JSON.stringify(renderStateQueries(process.argv[3]))};\n`,
);
writeFileSync(
  join(destination, 'sql.ts'),
  `// Generated from SQL files and Nunjucks templates.\nexport const memoryModule = ${JSON.stringify(sql('guinea/memory.sql'))};\nexport const queries = ${JSON.stringify(renderQueries(process.argv[3]))};\nexport const capabilitiesSql = ${JSON.stringify(renderCapabilities(process.argv[3]))};\n`,
);
writeFileSync(
  join(destination, 'event-ranking-sql.ts'),
  `// Generated from SQL template.\nexport const eventRankingSql = ${JSON.stringify(renderEventRanking(process.argv[3]))};\n`,
);
writeFileSync(
  join(destination, 'event-tracks-sql.ts'),
  `// Generated from SQL file.\nexport const eventTracksSql = ${JSON.stringify(sql('event_tracks.sql'))};\n`,
);

// Configure the stock embedder's default plugin list; lifecycle code is unchanged.
const defaultsPath = join(checkout, 'ui/src/core/embedder/default_plugins.ts');
const defaults = readFileSync(defaultsPath, 'utf8');
if (!defaults.includes("'rs.guinea.Profile'")) {
  const marker = 'export const defaultPlugins = [';
  if (!defaults.includes(marker))
    throw new Error('Pinned Perfetto default plugin configuration changed');
  writeFileSync(
    defaultsPath,
    defaults.replace(marker, `${marker}\n  'rs.guinea.Profile',`),
  );
}

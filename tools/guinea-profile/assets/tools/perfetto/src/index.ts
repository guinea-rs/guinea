import m from 'mithril';
import type {PerfettoPlugin} from '../../public/plugin';
import type {Trace} from '../../public/trace';
import {TrackNode} from '../../public/workspace';
import {Button} from '../../widgets/button';
import {NUM, STR} from '../../trace_processor/query_result';
import {Dashboard} from '../dev.perfetto.DataExplorer/dashboard/dashboard';
import type {
  DashboardDataSource,
  DashboardBrushFilter,
  DashboardItem,
} from '../dev.perfetto.DataExplorer/dashboard/dashboard_registry';
import '../dev.perfetto.DataExplorer/styles.scss';
import {datasets, sectionsFor} from './catalog';
import {parseMetrics} from './model';
import {prepareDatasets} from './session';
import {memoryModule, capabilitiesSql} from './sql';
import {EventRanking} from './event-ranking';
import {eventTracksSql} from './event-tracks-sql';
import {eventTracks} from './workspace';
import {registerStateChanges} from './state-changes';

export default class implements PerfettoPlugin {
  static readonly id = 'rs.guinea.Profile';

  async onTraceLoad(trace: Trace): Promise<void> {
    await registerStateChanges(trace);
    const capabilities = await trace.engine.query(capabilitiesSql);
    if (capabilities.error()) throw new Error(capabilities.error());
    const metrics: string[] = [];
    for (const row = capabilities.iter({metric: STR}); row.valid(); row.next())
      metrics.push(row.metric);
    const available = parseMetrics(metrics);
    if (available.length === 0) return;
    const prepared = await prepareDatasets(
      trace.engine,
      available.map((id) => datasets[id]),
      memoryModule,
    );
    const sources: DashboardDataSource[] = prepared.map((source) => ({
      name: source.id,
      nodeId: source.id,
      graphId: 'guinea-profile',
      tableName: source.tableName,
      columns: source.columns.map((name) => ({name})),
    }));
    const sections = sectionsFor(available);
    const filters = new Map<string, DashboardBrushFilter[]>();
    let selected =
      sections.find((section) => section.id === 'events') ?? sections[0];
    trace.pages.registerPage({
      route: '/guinea_profile',
      render: () =>
        m(
          'div',
          {
            style: {
              height: '100%',
              minHeight: '0',
              minWidth: '0',
              overflow: 'hidden',
              display: 'flex',
              flexDirection: 'column',
              boxSizing: 'border-box',
            },
          },
          [
            m(
              'div',
              {
                style: {
                  padding: '8px',
                  display: 'flex',
                  gap: '8px',
                  flexShrink: '0',
                },
              },
              sections.map((section) =>
                m(Button, {
                  label: section.title,
                  active: section === selected,
                  onclick: () => {
                    selected = section;
                    filters.clear();
                  },
                }),
              ),
            ),
            selected
              ? m(
                  'div',
                  {
                    style: {
                      flex: '1',
                      minHeight: '0',
                      minWidth: '0',
                      // Dashboard owns its canvas scrolling. Events have no
                      // dashboard canvas, so only their body scrolls.
                      overflow: selected.id === 'events' ? 'auto' : 'hidden',
                    },
                  },
                  selected.id === 'events'
                    ? m(EventRanking, {trace})
                    : [
                        m(
                          'div',
                          {
                            style: {
                              height: '100%',
                            },
                          },
                          m(Dashboard, {
                            dashboardId: selected.id,
                            trace,
                            items: selected.items,
                            sources,
                            brushFilters: filters,
                            onItemsChange: (items: DashboardItem[]) => {
                              if (selected) selected.items = items;
                            },
                            onBrushFiltersChange: (
                              updated: Map<string, DashboardBrushFilter[]>,
                            ) => {
                              filters.clear();
                              for (const [id, value] of updated)
                                filters.set(id, value);
                            },
                          }),
                        ),
                      ],
                )
              : m('p', 'This recording contains no Guinea metrics.'),
          ],
        ),
    });
    trace.sidebar.addMenuItem({
      section: 'current_trace',
      text: 'Guinea',
      href: '#!/guinea_profile',
      icon: 'dashboard',
    });
    trace.initialPage.suggest('/guinea_profile', 200);
    trace.onTraceReady.addListener(async () => {
      const result = await trace.engine.query(eventTracksSql);
      if (result.error()) throw new Error(result.error());
      const events = new Map<number, string>();
      for (
        const row = result.iter({track_id: NUM, name: STR});
        row.valid();
        row.next()
      ) {
        events.set(row.track_id, row.name);
      }
      const workspace = trace.workspaces.createEmptyWorkspace('Guinea');
      for (const track of eventTracks(trace.tracks.getAllTracks(), events)) {
        workspace.addChildLast(new TrackNode(track));
      }
      for (const node of trace.defaultWorkspace.flatTracks) {
        if (!node.uri) continue;
        if (node.uri.endsWith('/flamechart')) {
          workspace.addChildLast(
            new TrackNode({
              uri: node.uri,
              name: node.parent?.name ?? 'CPU flamechart',
            }),
          );
        } else if (
          /^(Rust heap|Guinea scopes|Tracing|Process) \/ /.test(node.name)
        ) {
          workspace.addChildLast(
            new TrackNode({uri: node.uri, name: node.name}),
          );
        }
      }
      if (workspace.children.length > 0)
        trace.workspaces.switchWorkspace(workspace);
    });
  }
}

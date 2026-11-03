import m from 'mithril';
import type {Trace} from '../../public/trace';
import {AsyncMemo} from '../../base/async_memo';
import {Memo} from '../../base/memo';
import {NUM, NUM_NULL, STR, STR_NULL} from '../../trace_processor/query_result';
import {Tree, TreeNode} from '../../widgets/tree';
import {Button} from '../../widgets/button';
import {stateSql} from './state-sql';
import {eventTracksSql} from './event-tracks-sql';
import {
  parseStatePayload,
  stateSelectionQuery,
  type Capture,
  type StatePayload,
} from './state-model';

interface StateRow {
  slice_id: number;
  update_id: number | null;
  reducer: string;
  update_type: string;
  revision: number;
  scope_id: number;
  change: string | null;
  cause_slice_id: number | null;
  total_count: number;
  payload: string | null;
}

function jsonTree(value: unknown, key = 'value'): m.Children {
  if (value !== null && typeof value === 'object') {
    return m(
      TreeNode,
      {
        left: key,
        summary: Array.isArray(value) ? `${value.length} items` : 'object',
        startsCollapsed: true,
      },
      Object.entries(value).map(([key, value]) => jsonTree(value, key)),
    );
  }
  return m(TreeNode, {left: key, right: JSON.stringify(value)});
}

function captured(value: Capture): m.Children {
  if (value.status === 'json') return m(Tree, jsonTree(value.value));
  const labels = {
    unavailable: 'Serialization is unavailable.',
    limit_exceeded: 'Capture exceeded its size limit.',
    failed: 'Serialization failed.',
  };
  return m('p', [
    labels[value.status],
    value.status === 'failed' ? ` ${value.value}` : '',
  ]);
}

class StateDetail implements m.ClassComponent<{row: StateRow; trace: Trace}> {
  private readonly parsed = new Memo<StatePayload | undefined>();
  private pane: 'diff' | 'update' | 'before' | 'after' = 'diff';
  view({attrs: {row, trace}}: m.Vnode<{row: StateRow; trace: Trace}>) {
    let payload: StatePayload | undefined;
    try {
      payload = this.parsed.use({
        key: row.slice_id,
        compute: () => parseStatePayload(row.payload),
      });
      if (payload && payload.update_id !== row.update_id)
        throw new Error('State payload identity mismatch');
    } catch (error) {
      return m('p', String(error));
    }
    return m('section', {style: {padding: '12px'}}, [
      m('h3', `${row.reducer} · revision ${row.revision}`),
      m(Tree, [
        m(TreeNode, {left: 'Update', right: row.update_type}),
        m(TreeNode, {left: 'Scope', right: String(row.scope_id)}),
        m(TreeNode, {left: 'Change', right: row.change ?? 'not_captured'}),
      ]),
      row.cause_slice_id !== null &&
        m(Button, {
          label: 'Go to cause',
          onclick: () => {
            trace.selection.selectSqlEvent('slice', row.cause_slice_id!, {
              scrollToSelection: true,
            });
          },
        }),
      payload
        ? [
            m(
              'div',
              {style: {display: 'flex', gap: '8px', margin: '12px 0'}},
              (['diff', 'update', 'before', 'after'] as const).map((pane) =>
                m(Button, {
                  label: pane[0].toUpperCase() + pane.slice(1),
                  active: this.pane === pane,
                  onclick: () => {
                    this.pane = pane;
                  },
                }),
              ),
            ),
            this.pane === 'diff' && payload.diff.status === 'json'
              ? (payload.diff.value as unknown[]).length === 0
                ? m(
                    'p',
                    'State unchanged: the update was applied, but captured values are identical.',
                  )
                : m(
                    Tree,
                    (
                      payload.diff.value as {
                        op: string;
                        path: string;
                        value?: unknown;
                      }[]
                    ).map((operation) =>
                      m(
                        TreeNode,
                        {
                          left: `${operation.op} ${operation.path || '(root)'}`,
                          startsCollapsed: false,
                        },
                        Object.hasOwn(operation, 'value')
                          ? jsonTree(operation.value, 'value')
                          : undefined,
                      ),
                    ),
                  )
              : captured(payload[this.pane]),
          ]
        : m(
            'p',
            'Values were not captured in normal mode. Enable detailed recording for subsequent updates.',
          ),
    ]);
  }
}

export async function registerStateChanges(trace: Trace): Promise<void> {
  const presence = await trace.engine.query(stateSql.presence);
  if (presence.error()) throw new Error(presence.error());
  const row = presence.iter({present: NUM});
  if (!row.valid() || row.present === 0) return;
  const prepared = await trace.engine.query(stateSql.module);
  if (prepared.error()) throw new Error(prepared.error());
  const tracks = await trace.engine.query(eventTracksSql);
  if (tracks.error()) throw new Error(tracks.error());
  const eventTrackIds = new Set<number>();
  for (const row = tracks.iter({track_id: NUM}); row.valid(); row.next())
    eventTrackIds.add(row.track_id);
  const memo = new AsyncMemo<StateRow[]>();
  trace.selection.registerTrackEventSelectionTab({
    id: 'rs.guinea.StateChange',
    name: 'State change',
    priority: 100,
    render(selection) {
      if (
        !trace.tracks
          .getTrack(selection.trackUri)
          ?.tags?.trackIds?.some((id) => eventTrackIds.has(id))
      )
        return undefined;
      const result = memo.use({
        key: {id: selection.eventId},
        compute: async () => {
          const query = await trace.engine.query(
            stateSelectionQuery(stateSql.selection, selection.eventId),
          );
          if (query.error()) throw new Error(query.error());
          const rows: StateRow[] = [];
          for (
            const row = query.iter({
              slice_id: NUM,
              update_id: NUM_NULL,
              reducer: STR,
              update_type: STR,
              revision: NUM,
              scope_id: NUM,
              change: STR_NULL,
              cause_slice_id: NUM_NULL,
              total_count: NUM,
              payload: STR_NULL,
            });
            row.valid();
            row.next()
          ) {
            rows.push({
              slice_id: row.slice_id,
              update_id: row.update_id,
              reducer: row.reducer,
              update_type: row.update_type,
              revision: row.revision,
              scope_id: row.scope_id,
              change: row.change,
              cause_slice_id: row.cause_slice_id,
              total_count: row.total_count,
              payload: row.payload,
            });
          }
          return rows;
        },
      });
      if ('error' in result && result.error)
        return {isLoading: false, content: m('p', String(result.error))};
      if (!result.data) return {isLoading: true, content: undefined};
      if (result.data.length === 0) return undefined;
      return {
        isLoading: result.isPending,
        content: [
          result.data[0].total_count > result.data.length &&
            m(
              'p',
              `Showing the first ${result.data.length} of ${result.data[0].total_count} updates. Select an individual state event to inspect it.`,
            ),
          result.data.map((row) =>
            m(StateDetail, {key: row.update_id ?? row.slice_id, row, trace}),
          ),
        ],
      };
    },
  });
}

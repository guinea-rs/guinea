import m from 'mithril';
import type {Trace} from '../../public/trace';
import {NUM, STR} from '../../trace_processor/query_result';
import {EChartView} from '../../components/widgets/charts/echart_view';
import {eventRankingSql} from './event-ranking-sql';
import {DataGrid} from '../../components/widgets/datagrid/datagrid';
import {SQLDataSource} from '../../components/widgets/datagrid/sql_data_source';

export class EventRanking implements m.ClassComponent<{trace: Trace}> {
  private rows?: {event: string; allocated_mib: number}[];
  private error?: string;
  private counts?: SQLDataSource;

  async oninit({attrs}: m.Vnode<{trace: Trace}>) {
    this.counts = new SQLDataSource({
      engine: attrs.trace.engine,
      tableOrSubquery: 'guinea_profile_events',
      columns: {event: {}, occurrences: {}, allocated_mib: {}},
    });
    try {
      const result = await attrs.trace.engine.query(eventRankingSql);
      if (result.error()) throw new Error(result.error());
      this.rows = [];
      for (
        const row = result.iter({event: STR, allocated_mib: NUM});
        row.valid();
        row.next()
      ) {
        this.rows.push({event: row.event, allocated_mib: row.allocated_mib});
      }
    } catch (error) {
      this.error = String(error);
    }
    m.redraw();
  }

  view() {
    const rows = this.rows;
    return m('section', {style: {padding: '16px', overflowX: 'auto'}}, [
      m(
        'h2',
        {style: {fontSize: '16px', margin: '0 0 12px'}},
        'Event totals · entire recording',
      ),
      this.counts &&
        m(DataGrid, {
          data: this.counts,
          schema: {
            event: {title: 'Event', columnType: 'text'},
            occurrences: {title: 'Count', columnType: 'quantitative'},
            allocated_mib: {
              title: 'Allocated (MiB)',
              columnType: 'quantitative',
            },
          },
        }),
      m(
        'h2',
        {style: {fontSize: '16px', margin: '0 0 12px'}},
        'Top allocation events · MiB',
      ),
      this.error
        ? m('p', this.error)
        : m(
            'div',
            {style: {minWidth: '1000px'}},
            m(EChartView, {
              height: rows ? Math.max(260, rows.length * 52 + 60) : 260,
              empty: rows?.length === 0,
              option: rows && {
                animation: false,
                grid: {
                  left: 510,
                  right: 100,
                  top: 12,
                  bottom: 40,
                  containLabel: false,
                },
                xAxis: {
                  type: 'value',
                  name: 'Allocated (MiB)',
                  nameLocation: 'middle',
                  nameGap: 26,
                },
                yAxis: {
                  type: 'category',
                  inverse: true,
                  data: rows.map((row) => row.event),
                  axisLabel: {
                    fontSize: 12,
                    width: 480,
                    overflow: 'break',
                    interval: 0,
                    margin: 14,
                  },
                  axisTick: {show: false},
                  axisLine: {show: false},
                },
                tooltip: {trigger: 'axis', confine: true},
                series: [
                  {
                    type: 'bar',
                    barMaxWidth: 22,
                    data: rows.map((row) => row.allocated_mib),
                    label: {
                      show: true,
                      position: 'right',
                      fontSize: 12,
                      formatter: (item: {value?: unknown}) =>
                        Number(item.value).toLocaleString('en-US', {
                          maximumFractionDigits: 2,
                        }),
                    },
                  },
                ],
              },
            }),
          ),
    ]);
  }
}

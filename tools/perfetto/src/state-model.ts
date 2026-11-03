export interface Capture {
  status: 'json' | 'unavailable' | 'limit_exceeded' | 'failed';
  value?: unknown;
}
export interface StatePayload {
  schema_version: 1;
  update_id: number;
  before: Capture;
  update: Capture;
  after: Capture;
  diff: Capture;
}

export function parseStatePayload(
  text: string | null,
): StatePayload | undefined {
  if (text === null) return undefined;
  const data = JSON.parse(text);
  if (data?.schema_version !== 1 || !Number.isSafeInteger(data.update_id))
    throw new Error('Unsupported state payload');
  for (const key of ['before', 'update', 'after', 'diff']) {
    const value = data[key];
    if (
      !value ||
      !['json', 'unavailable', 'limit_exceeded', 'failed'].includes(
        value.status,
      ) ||
      (value.status === 'json' && !Object.hasOwn(value, 'value'))
    )
      throw new Error(`Invalid state capture: ${key}`);
  }
  if (data.diff.status === 'json' && !Array.isArray(data.diff.value))
    throw new Error('Invalid state patch');
  return data;
}

export function stateSelectionQuery(template: string, eventId: number): string {
  if (!Number.isSafeInteger(eventId) || eventId < 0)
    throw new Error('Invalid event id');
  return template.replaceAll('__EVENT_ID__', String(eventId));
}

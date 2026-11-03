/** Match trace tracks by their SQL ids, never by formatted UI labels. */
export function eventTracks(
  tracks: readonly {uri: string; tags?: {trackIds?: readonly number[]}}[],
  names: ReadonlyMap<number, string>,
): {uri: string; name: string}[] {
  return tracks.flatMap((track) => {
    const id = track.tags?.trackIds?.find((id) => names.has(id));
    return id === undefined
      ? []
      : [{uri: track.uri, name: `Events · ${names.get(id)}`}];
  });
}

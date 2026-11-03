-- Select by recorded semantics, independent of UI names and thread grouping.
SELECT DISTINCT t.id AS track_id, COALESCE(t.name, 'Guinea events') AS name
FROM track t JOIN slice s ON s.track_id = t.id
WHERE EXTRACT_ARG(s.arg_set_id, 'debug.guinea_kind') IS NOT NULL
ORDER BY t.id;

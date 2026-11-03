# Refresh packaged copies from their canonical repository sources.
$ErrorActionPreference = 'Stop'
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$assets = Join-Path $PSScriptRoot 'assets'
foreach ($directory in @('src', 'scripts', 'sql')) {
    $source = Join-Path $repository "tools/perfetto/$directory"
    $destination = Join-Path $assets "tools/perfetto/$directory"
    # This target is fixed inside this package; remove stale copied files too.
    if (Test-Path -LiteralPath $destination) {
        $resolved = (Resolve-Path -LiteralPath $destination).Path
        if (!$resolved.StartsWith((Resolve-Path -LiteralPath $assets).Path + [IO.Path]::DirectorySeparatorChar)) { throw 'Invalid assets target' }
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
    Copy-Item -LiteralPath $source -Destination $destination -Recurse
}
foreach ($name in @('build.sh', 'package.json', 'package-lock.json')) {
    Copy-Item -LiteralPath (Join-Path $repository "tools/perfetto/$name") -Destination (Join-Path $assets "tools/perfetto/$name")
}
foreach ($name in @('top_cpu_actions.sql', 'event_tracks.sql', 'guinea/memory.sql')) {
    Copy-Item -LiteralPath (Join-Path $repository "crates/guinea-trace/sql/$name") -Destination (Join-Path $assets "crates/guinea-trace/sql/$name")
}

#!/usr/bin/env bash
set -euo pipefail
# macOS does not ship these GNU/Linux build prerequisites.
for tool in flock sha256sum; do
  if ! command -v "$tool" >/dev/null 2>&1; then
    echo "Perfetto UI build requires $tool on PATH. Install GNU coreutils (sha256sum, sort) and a flock implementation, or use guinea-profile --ui-dir <prebuilt-dist> view <recording>. Windows users must install these in WSL." >&2
    exit 1
  fi
done
if ! printf '' | sort -z >/dev/null 2>&1; then
  echo 'Perfetto UI build requires sort with -z support. Put GNU coreutils on PATH, or use guinea-profile --ui-dir <prebuilt-dist> view <recording>.' >&2
  exit 1
fi
integration="$(cd "$(dirname "$0")" && pwd)"
revision=79997ada96b2587556cfc3851434407fbed289b1
checkout="$HOME/.cache/guinea-profile/perfetto-$revision"
mkdir -p "$(dirname "$checkout")"
# Agents sharing the tool cache must not build into the same tree concurrently.
exec 9>"$checkout.lock"
flock 9
fingerprint="$(find "$integration/src" "$integration/scripts" "$integration/sql" -type f -print0 | sort -z | xargs -0 sha256sum; sha256sum "$integration/build.sh" "$integration/package-lock.json" "$integration/../../crates/guinea-trace/sql/top_cpu_actions.sql" "$integration/../../crates/guinea-trace/sql/event_tracks.sql" "$integration/../../crates/guinea-trace/sql/guinea/memory.sql")"
dist="$checkout/out/guinea/ui/dist"
if [ -f "$checkout/guinea-build-fingerprint" ] && [ -f "$dist/index.html" ] && [ "$(cat "$checkout/guinea-build-fingerprint")" = "$fingerprint" ]; then
  if command -v wslpath >/dev/null; then wslpath -w "$dist"; else printf "%s\n" "$dist"; fi
  exit 0
fi
mkdir -p "$(dirname "$checkout")"
if [ ! -d "$checkout/.git" ]; then
  git -c core.autocrlf=false -c core.eol=lf init "$checkout" >&2
  git -C "$checkout" remote add origin https://github.com/google/perfetto.git
  git -C "$checkout" fetch --depth 1 origin "$revision" >&2
  git -c core.autocrlf=false -c core.eol=lf -C "$checkout" checkout --detach FETCH_HEAD >&2
fi
[ "$(git -C "$checkout" rev-parse HEAD)" = "$revision" ] || { echo 'Wrong Perfetto revision in tool cache' >&2; exit 1; }
cd "$checkout"
# Keep downloaded shell/Python launchers executable on Linux.
git config core.autocrlf false
git config core.eol lf
python3 tools/install-build-deps --ui --no-dev-tools --public-npm >&2
mkdir -p "$checkout/guinea-integration-deps"
cp "$integration/package.json" "$integration/package-lock.json" "$checkout/guinea-integration-deps/"
ui/npm ci --prefix "$checkout/guinea-integration-deps" --ignore-scripts --no-audit --no-fund >&2
ui/node --experimental-strip-types "$integration/scripts/prepare.ts" "$checkout" "$checkout/guinea-integration-deps/package.json" >&2
tools/gn gen --args='is_debug=false skip_buildtools_check=true' out/guinea >&2
tools/ninja -C out/guinea -j 8 traceconv_wasm proto_utils_wasm trace_processor_memory64_wasm >&2
ui/node ui/build.mjs --no-wasm --no-depscheck --only-wasm-memory64 --out out/guinea >&2
printf '%s' "$fingerprint" > "$checkout/guinea-build-fingerprint"
if command -v wslpath >/dev/null; then wslpath -w "$dist"; else printf "%s\n" "$dist"; fi

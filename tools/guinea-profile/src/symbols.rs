//! Resolve recorded module-relative addresses with Samply's symbol library.
use super::Result;
use serde_json::{Value, json};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use wholesym::{LookupAddress, SymbolManager, SymbolManagerConfig, debugid::DebugId};

#[derive(Clone)]
pub struct Options {
    pub executable: Option<PathBuf>,
    pub directories: Vec<PathBuf>,
    pub allow_unresolved: bool,
    pub timeout: Duration,
}

fn addresses(profile: &Value) -> Vec<BTreeSet<u32>> {
    let mut result = vec![BTreeSet::new(); profile["libs"].as_array().map_or(0, Vec::len)];
    for thread in profile["threads"].as_array().into_iter().flatten() {
        let mut visited = BTreeSet::new();
        for mut stack in thread["samples"]["stack"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
        {
            while visited.insert(stack) {
                let Some(frame) = thread["stackTable"]["frame"][stack as usize].as_u64() else {
                    break;
                };
                let f = &thread["frameTable"];
                let func = f["func"][frame as usize].as_u64().unwrap_or(u64::MAX) as usize;
                if let Some(resource) = thread["funcTable"]["resource"]
                    .get(func)
                    .and_then(Value::as_u64)
                    && let (Some(lib), Some(address)) = (
                        thread["resourceTable"]["lib"][resource as usize].as_u64(),
                        f["address"][frame as usize]
                            .as_u64()
                            .and_then(|v| u32::try_from(v).ok()),
                    )
                        && let Some(addresses) = result.get_mut(lib as usize) {
                            addresses.insert(address);
                        }
                let Some(parent) = thread["stackTable"]["prefix"][stack as usize].as_u64() else {
                    break;
                };
                stack = parent;
            }
        }
    }
    result
}

fn is_primary(lib: &Value, options: &Options) -> bool {
    options.executable.as_ref().is_some_and(|path| {
        path.file_name().is_some_and(|name| {
            lib["name"]
                .as_str()
                .is_some_and(|recorded| name.to_string_lossy().eq_ignore_ascii_case(recorded))
        })
    })
}

pub fn resolve(
    profile: &Value,
    seed: Option<Value>,
    output: &Path,
    cache: &Path,
    options: &Options,
) -> Result<Value> {
    tokio::runtime::Runtime::new()?.block_on(resolve_async(profile, seed, output, cache, options))
}

async fn resolve_async(
    profile: &Value,
    seed: Option<Value>,
    output: &Path,
    cache: &Path,
    options: &Options,
) -> Result<Value> {
    let mut config = SymbolManagerConfig::default()
        .respect_nt_symbol_path(true)
        .default_nt_symbol_path(format!(
            "srv*{}*https://msdl.microsoft.com/download/symbols",
            cache.join("symbols").display()
        ));
    for directory in &options.directories {
        config = config.extra_symbol_directory(directory);
    }
    if let Some(parent) = options.executable.as_ref().and_then(|p| p.parent()) {
        config = config.extra_symbol_directory(parent);
    }
    let manager = SymbolManager::with_config(config);
    let mut symbols = seed.unwrap_or_else(|| json!({"string_table":[],"data":[]}));
    if !symbols["string_table"].is_array() || !symbols["data"].is_array() {
        return Err("invalid symbol sidecar schema".into());
    }
    let collected = addresses(profile);
    let mut report = Vec::new();
    let mut primary_failed = false;
    let modules = profile["libs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut order: Vec<_> = (0..modules.len()).collect();
    order.sort_by_key(|index| !is_primary(&modules[*index], options));
    let started = std::time::Instant::now();
    for index in order {
        let lib = &modules[index];
        let wanted = &collected[index];
        if wanted.is_empty() {
            continue;
        }
        let name = lib["name"].as_str().unwrap_or("[unknown]");
        let primary = is_primary(lib, options);
        let debug_name = lib["debugName"].as_str().unwrap_or(name);
        let expected = lib["breakpadId"]
            .as_str()
            .and_then(|id| DebugId::from_breakpad(id).ok());
        let existing = symbols["data"].as_array().unwrap().iter().position(|data| {
            data["debug_name"] == lib["debugName"] && data["code_id"] == lib["codeId"]
        });
        let entry_index = match existing {
            Some(i) => i,
            None => {
                let data = symbols["data"].as_array_mut().unwrap();
                data.push(json!({"debug_name":lib["debugName"],"code_id":lib["codeId"],"known_addresses":[],"symbol_table":[]}));
                data.len() - 1
            }
        };
        let known: BTreeSet<u32> = symbols["data"][entry_index]["known_addresses"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v[0].as_u64().and_then(|n| u32::try_from(n).ok()))
            .collect();
        let missing: Vec<_> = wanted.difference(&known).copied().collect();
        let mut resolved = wanted.intersection(&known).count();
        let mut problem = None;
        if !missing.is_empty() {
            if let Some(expected) = expected {
                let binary = if primary {
                    options.executable.clone()
                } else {
                    lib["path"].as_str().map(PathBuf::from)
                };
                let load = async {
                    if let Some(binary) = binary.as_ref().filter(|path| path.is_file())
                        && let Ok(map) = manager
                            .load_symbol_map_for_binary_at_path(binary, None)
                            .await
                            && map.debug_id() == expected {
                                return Ok(map);
                            }
                    manager.load_symbol_map(debug_name, expected).await
                };
                match tokio::time::timeout(
                    options
                        .timeout
                        .saturating_sub(started.elapsed())
                        .min(Duration::from_secs(if primary { 60 } else { 10 })),
                    load,
                )
                .await
                {
                    Ok(Ok(map)) if map.debug_id() == expected => {
                        for address in missing {
                            if let Some(info) = map.lookup_sync(LookupAddress::Relative(address)) {
                                let string_index =
                                    symbols["string_table"].as_array().unwrap().len();
                                symbols["string_table"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(json!(map.resolve_symbol_name(info.symbol.name)));
                                let entry = &mut symbols["data"][entry_index];
                                let symbol_index = entry["symbol_table"].as_array().unwrap().len();
                                entry["symbol_table"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(json!({"symbol":string_index}));
                                entry["known_addresses"]
                                    .as_array_mut()
                                    .unwrap()
                                    .push(json!([address, symbol_index]));
                                resolved += 1;
                            }
                        }
                    }
                    Ok(Ok(_)) => problem = Some("symbol file identity mismatch".to_owned()),
                    Ok(Err(error)) => problem = Some(error.to_string()),
                    Err(_) => problem = Some("symbol lookup timed out".to_owned()),
                }
            } else {
                problem = Some("recording has no valid module debug identity".to_owned());
            }
        }
        let unresolved = wanted.len() - resolved;
        eprintln!(
            "Symbols: {name}: {resolved}/{} addresses resolved{}",
            wanted.len(),
            if unresolved > 0 { " (partial)" } else { "" }
        );
        if primary && unresolved > 0 {
            primary_failed = true;
        }
        report.push(json!({"module":name,"debug_name":debug_name,"debug_id":lib["breakpadId"],"primary":primary,
            "requested":wanted.len(),"resolved":resolved,"unresolved":unresolved,"error":problem}));
    }
    fs::write(
        output.join("symbolication.json"),
        serde_json::to_vec_pretty(
            &json!({"schema_version":1,"modules":report,"allow_unresolved":options.allow_unresolved}),
        )?,
    )?;
    if primary_failed && !options.allow_unresolved {
        return Err("application symbols are missing or incomplete; see symbolication.json, supply the matching executable/PDB with --executable/--symbol-dir, or explicitly use --allow-unresolved".into());
    }
    fs::write(
        output.join("cpu.resolved.syms.json"),
        serde_json::to_vec(&symbols)?,
    )?;
    Ok(symbols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(windows, target_env = "msvc"))]
    #[test]
    fn resolves_a_real_matching_pdb_without_a_seed_sidecar() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("fixture.rs");
        let exe = temp.path().join("fixture.exe");
        fs::write(&source, r#"#[no_mangle] #[inline(never)] pub extern "C" fn guinea_profile_fixture_symbol() -> u64 { std::hint::black_box(42) } fn main() { println!("{}", guinea_profile_fixture_symbol()); }"#).unwrap();
        let built = std::process::Command::new("rustc")
            .arg(&source)
            .args(["--edition=2021", "-g", "-C", "link-arg=/DEBUG:FULL", "-o"])
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{}",
            String::from_utf8_lossy(&built.stderr)
        );
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let (info, address) = runtime.block_on(async {
            let info = SymbolManager::library_info_for_binary_at_path(&exe, None)
                .await
                .unwrap();
            let manager = SymbolManager::with_config(
                SymbolManagerConfig::default().extra_symbol_directory(temp.path()),
            );
            let map = manager
                .load_symbol_map_for_binary_at_path(&exe, None)
                .await
                .unwrap();
            let names: Vec<_> = map
                .iter_symbols()
                .filter(|(_, name)| name.contains("fixture::guinea_profile_fixture_symbol()"))
                .map(|(a, n)| (a, n.into_owned()))
                .collect();
            assert_eq!(
                names.len(),
                1,
                "fixture candidates: {names:?}, symbol count {}",
                map.symbol_count()
            );
            let address = names[0].0;

            (info, address)
        });
        let profile = json!({"libs":[{"name":"fixture.exe","debugName":info.debug_name,
            "breakpadId":info.debug_id.unwrap().breakpad().to_string(),"codeId":"fixture-test","path":exe}],
            "threads":[{"samples":{"stack":[0]},"stackTable":{"frame":[0],"prefix":[null]},
            "frameTable":{"func":[0],"address":[address]},"funcTable":{"resource":[0]},"resourceTable":{"lib":[0]}}]});
        let options = Options {
            executable: Some(exe),
            directories: vec![],
            allow_unresolved: false,
            timeout: Duration::from_secs(10),
        };
        let resolved = resolve(&profile, None, temp.path(), temp.path(), &options).unwrap();
        assert_eq!(
            resolved["string_table"][0],
            "fixture::guinea_profile_fixture_symbol()"
        );
        assert_eq!(resolved["data"][0]["known_addresses"][0][0], address);
        let report: Value = serde_json::from_reader(
            fs::File::open(temp.path().join("symbolication.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(report["modules"][0]["resolved"], 1);
        assert_eq!(report["modules"][0]["unresolved"], 0);
        assert_eq!(report["modules"][0]["error"], Value::Null);
    }

    #[test]
    fn collects_referenced_module_relative_addresses_and_shared_prefixes_once() {
        let p = json!({"libs":[{},{}],"threads":[{"samples":{"stack":[1,1,null]},
            "stackTable":{"frame":[0,1,2],"prefix":[null,0,null]},
            "frameTable":{"func":[0,1,2],"address":[64,128,999]},
            "funcTable":{"resource":[0,1,0]},"resourceTable":{"lib":[0,1]}}]});
        assert_eq!(
            addresses(&p),
            vec![BTreeSet::from([64]), BTreeSet::from([128])]
        );
    }
}

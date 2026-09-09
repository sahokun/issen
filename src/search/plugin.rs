use std::path::{Path, PathBuf};

use issen_plugin_api::{BoxedRGuiToolProvider, BoxedRSearchProvider, PluginModuleRef, RAction};

use super::{Action, SearchProvider, SearchResult};

/// The maximum score `fuzzy_match` can award per character (14, for a
/// boundary + consecutive match — the same value `app_index.rs`'s
/// `LOW_PRIORITY_PENALTY` is derived from). Plugin-supplied `score` values
/// aren't trusted and get clamped to this range, so a third-party plugin
/// can't return something like `i32::MAX` and permanently camp the top of
/// the result list.
const FUZZY_MAX_SCORE_PER_CHAR: i32 = 14;

fn clamp_plugin_score(query: &str, score: i32) -> i32 {
    let max = FUZZY_MAX_SCORE_PER_CHAR * query.chars().count().max(1) as i32;
    score.clamp(-max, max)
}

/// Loads `*.dll` files under `%APPDATA%\Issen\plugins` as search provider
/// plugins.
///
/// *Rationale:* deliberately not using `abi_stable`'s high-level
/// `RootModule::load_from_file`. `RootModuleStatics` caches **one instance
/// per root-module Rust type**, process-wide (see `abi_stable`'s
/// `library/root_mod_trait.rs`) — so loading multiple different plugin
/// DLLs that share the same `PluginModuleRef` type would make every
/// `load_from_file` after the first ignore its path argument and just
/// return the first plugin loaded. Instead, this uses the low-level
/// `lib_header_from_path` + `LibHeader::init_root_module` API directly,
/// which loads each file independently without going through that global
/// cache.
pub struct PluginProvider {
    loaded: Vec<BoxedRSearchProvider>,
    // No host-side launch UI exists yet for the third-party GUI tool
    // plugin mechanism, so only test code reads this today.
    #[allow(dead_code)]
    gui_tools: Vec<BoxedRGuiToolProvider>,
}

impl PluginProvider {
    pub fn load_from_app_data() -> Self {
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return Self {
                loaded: Vec::new(),
                gui_tools: Vec::new(),
            };
        };
        let dir = PathBuf::from(appdata)
            .join(crate::config::APP_NAME)
            .join("plugins");
        Self::load_from_dir(&dir)
    }

    /// The loaded GUI tool plugins. No host-side launch UI exists yet for
    /// these, so only test code calls this today.
    #[allow(dead_code)]
    pub fn gui_tools(&self) -> &[BoxedRGuiToolProvider] {
        &self.gui_tools
    }

    fn load_from_dir(dir: &Path) -> Self {
        let mut loaded = Vec::new();
        let mut gui_tools = Vec::new();
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Self { loaded, gui_tools };
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("dll") {
                continue;
            }
            match Self::load_one(&path) {
                Ok((provider, tools)) => {
                    loaded.push(provider);
                    gui_tools.extend(tools);
                }
                Err(err) => {
                    eprintln!("issen: failed to load plugin {}: {err}", path.display());
                }
            }
        }
        Self { loaded, gui_tools }
    }

    fn load_one(path: &Path) -> Result<(BoxedRSearchProvider, Vec<BoxedRGuiToolProvider>), String> {
        let header = abi_stable::library::lib_header_from_path(path).map_err(|e| e.to_string())?;
        let module: PluginModuleRef = header
            .init_root_module::<PluginModuleRef>()
            .map_err(|e| e.to_string())?;
        let provider = (module.new_provider())();
        // `gui_tools` is `#[sabi(missing_field(option))]`, so it comes back
        // `None` for an (older) plugin DLL that doesn't implement this
        // field (see issen-plugin-api).
        let gui_tools = module
            .gui_tools()
            .map(|f| f().into_iter().collect())
            .unwrap_or_default();
        Ok((provider, gui_tools))
    }

    fn convert(query: &str, result: issen_plugin_api::RSearchResult) -> SearchResult {
        SearchResult {
            title: result.title.into_string(),
            subtitle: result.subtitle.into_string(),
            action: match result.action {
                RAction::Launch { path, args } => Action::Launch {
                    path: PathBuf::from(path.into_string()),
                    args: args.into_string(),
                },
                RAction::LaunchUwp { aumid } => Action::LaunchUwp {
                    aumid: aumid.into_string(),
                },
                RAction::OpenUri(uri) => Action::OpenUri(uri.into_string()),
                RAction::CopyToClipboard(text) => Action::CopyToClipboard(text.into_string()),
            },
            score: clamp_plugin_score(query, result.score),
        }
    }
}

impl SearchProvider for PluginProvider {
    fn search(&self, query: &str) -> Vec<SearchResult> {
        let rquery = abi_stable::std_types::RStr::from(query);
        self.loaded
            .iter()
            .flat_map(|provider| provider.search(rquery).into_iter())
            .map(|result| Self::convert(query, result))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_directory_yields_no_providers() {
        let dir = std::env::temp_dir().join("issen-plugin-test-empty-dir-does-not-exist");
        let provider = PluginProvider::load_from_dir(&dir);
        assert!(provider.search("anything").is_empty());
    }

    #[test]
    fn clamps_plugin_score_to_fuzzy_match_bounds() {
        assert_eq!(clamp_plugin_score("ab", i32::MAX), 28);
        assert_eq!(clamp_plugin_score("ab", i32::MIN), -28);
        assert_eq!(clamp_plugin_score("ab", 10), 10);
    }

    /// Actually loads `issen-plugin-example` (a dummy plugin DLL that
    /// exists purely to exercise the ABI boundary — see
    /// `docs/architecture/plugins.md`). Assumes a full workspace build (`cargo
    /// build`/`cargo test` run from the repo root). In an environment
    /// where the DLL hasn't been built yet (e.g. `cargo test -p issen` run
    /// on its own), this detects the missing DLL and returns `None` to
    /// skip the test.
    fn load_test_plugin_provider() -> Option<PluginProvider> {
        let dll_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("debug")
            .join("issen_plugin_example.dll");
        if !dll_path.exists() {
            eprintln!(
                "issen_plugin_example.dll not built, skipping test: {}",
                dll_path.display()
            );
            return None;
        }
        let (provider, gui_tools) =
            PluginProvider::load_one(&dll_path).expect("failed to load the dummy plugin");
        Some(PluginProvider {
            loaded: vec![provider],
            gui_tools,
        })
    }

    /// Verifies the host and plugin DLL can exchange data using only
    /// strings and enums, never crossing a Rust type across the FFI
    /// boundary.
    #[test]
    fn loads_example_plugin_dll_and_converts_its_results() {
        let Some(provider) = load_test_plugin_provider() else {
            return;
        };

        let results = provider.search("example");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "Example Plugin Result");
        assert_eq!(results[0].subtitle, "issen-plugin-example");
        match &results[0].action {
            Action::OpenUri(uri) => {
                assert_eq!(uri.as_str(), "https://example.com/issen-plugin-example");
            }
            other => panic!("expected OpenUri, got {other:?}"),
        }
        // The plugin returns a fixed 100, which exceeds the clamp range
        // for the query "example" (7 chars, 14*7=98) and should be capped.
        assert_eq!(results[0].score, 98);

        assert!(provider.search("no-such-query-should-not-match").is_empty());
    }

    /// Verifies `RAction::CopyToClipboard` round-trips too, using the same
    /// DLL as `loads_example_plugin_dll_and_converts_its_results` (a
    /// different query triggers it, so the other test's expected result
    /// count doesn't change — see `plugin-example`'s `search_impl`).
    #[test]
    fn loads_example_plugin_dll_and_converts_copy_to_clipboard_result() {
        let Some(provider) = load_test_plugin_provider() else {
            return;
        };

        let results = provider.search("clipboard");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].title, "42");
        match &results[0].action {
            Action::CopyToClipboard(text) => assert_eq!(text, "42"),
            other => panic!("expected CopyToClipboard, got {other:?}"),
        }
    }

    /// Verifies that even a real panic inside the plugin doesn't take the
    /// host (this test process) down with it across the FFI boundary —
    /// `RSearchProvider::search`'s implementation catches it with
    /// `catch_unwind` (`plugin-example`). Also confirms that as long as
    /// the host stays alive, a panicking plugin's result is simply empty
    /// (`unwrap_or_default()`).
    #[test]
    fn panicking_plugin_does_not_crash_the_host() {
        let Some(provider) = load_test_plugin_provider() else {
            return;
        };

        let results = provider.search("panic");
        assert!(results.is_empty());

        // As proof the host (this test process) is still alive after the
        // panic, confirm the same provider still works for an ordinary
        // query.
        let results = provider.search("clipboard");
        assert_eq!(results.len(), 1);
    }

    /// Verifies `PluginModule::gui_tools` loads correctly across the FFI
    /// boundary, and that each tool's `RGuiToolMode::Standalone`/`Embedded`
    /// declaration round-trips as the enum it is (see `plugin-example`'s
    /// `StandaloneExampleTool`/`EmbeddedExampleTool`).
    #[test]
    fn loads_example_plugin_dll_and_lists_gui_tools() {
        let Some(provider) = load_test_plugin_provider() else {
            return;
        };

        let tools = provider.gui_tools();
        assert_eq!(tools.len(), 2);

        let names: Vec<String> = tools.iter().map(|t| t.name().into_string()).collect();
        assert!(names.contains(&"Example Standalone Tool".to_string()));
        assert!(names.contains(&"Example Embedded Tool".to_string()));

        let standalone = tools
            .iter()
            .find(|t| t.name().as_str() == "Example Standalone Tool")
            .expect("Standalone tool not found");
        assert_eq!(
            standalone.mode(),
            issen_plugin_api::RGuiToolMode::Standalone
        );

        let embedded = tools
            .iter()
            .find(|t| t.name().as_str() == "Example Embedded Tool")
            .expect("Embedded tool not found");
        assert_eq!(embedded.mode(), issen_plugin_api::RGuiToolMode::Embedded);
    }
}

pub mod alias;
pub mod app_index;
pub mod everything;
pub mod fuzzy;
pub mod plugin;
pub mod uwp;
pub mod windows_settings;

pub struct SearchResult {
    pub title: String,
    pub subtitle: String,
    pub action: Action,
    /// The fuzzy-match score, used to sort results across providers.
    pub score: i32,
}

#[derive(Debug)]
pub enum Action {
    /// `args` are the command alias's arguments. Every non-alias source (indexed
    /// apps, etc.) leaves this as an empty string.
    Launch {
        path: std::path::PathBuf,
        args: String,
    },
    LaunchUwp {
        aumid: String,
    },
    OpenUri(String),
    /// A result that copies text to the clipboard instead of launching anything
    /// (for plugins like a calculator, snippets, clipboard history, or translation —
    /// see the rationale on `RAction::CopyToClipboard` in `plugin-api`).
    CopyToClipboard(String),
}

pub trait SearchProvider {
    fn search(&self, query: &str) -> Vec<SearchResult>;
}

/// Builds a stable string from an `Action` that identifies results pointing at the
/// same launch target, so they can be treated as the same thing across providers.
/// Used both when registering an alias (`app.rs::register_selected_as_alias`) and by
/// the usage-history feature that boosts previously chosen results
/// (`crate::history`) to decide "is this the same result as before". `Action::Launch`
/// excludes `args` deliberately — every non-alias source always leaves it empty, and
/// multiple aliases pointing at the same executable are meant to be treated as one.
pub fn target_key(action: &Action) -> String {
    match action {
        Action::Launch { path, .. } => path.display().to_string(),
        Action::OpenUri(uri) => uri.clone(),
        Action::LaunchUwp { aumid } => aumid.clone(),
        Action::CopyToClipboard(text) => text.clone(),
    }
}

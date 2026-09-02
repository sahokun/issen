use std::path::PathBuf;

use super::fuzzy::fuzzy_match;
use super::{Action, SearchProvider, SearchResult};
use crate::config::AliasEntry;

pub struct AliasProvider<'a> {
    aliases: &'a [AliasEntry],
}

impl<'a> AliasProvider<'a> {
    pub fn new(aliases: &'a [AliasEntry]) -> Self {
        Self { aliases }
    }
}

impl SearchProvider for AliasProvider<'_> {
    fn search(&self, query: &str) -> Vec<SearchResult> {
        if query.is_empty() {
            return Vec::new();
        }

        self.aliases
            .iter()
            .filter_map(|alias| {
                fuzzy_match(query, &alias.name).map(|score| SearchResult {
                    title: alias.name.clone(),
                    subtitle: alias.target.clone(),
                    action: Action::Launch {
                        path: PathBuf::from(&alias.target),
                        args: alias.args.clone(),
                    },
                    score,
                })
            })
            .collect()
    }
}

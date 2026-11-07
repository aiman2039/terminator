//! The Explorer Contents search request.
use crate::{App, Job, preferences::ExplorerSearchMode, search};

impl App {
    /// Queue a Contents search when the query or filters changed. Results are
    /// tagged with a generation so stale workers are dropped.
    pub(crate) fn refresh_explorer_search(&mut self, cwd: &std::path::Path) {
        if self.preferences.explorer_search_mode != ExplorerSearchMode::Contents {
            return;
        }
        let query = search::Query {
            text: self.explorer_query.clone(),
            match_case: self.preferences.explorer_match_case,
            whole_word: self.preferences.explorer_whole_word,
            use_regex: self.preferences.explorer_regex,
            include: self.preferences.explorer_include.clone(),
            exclude: self.preferences.explorer_exclude.clone(),
        };
        let last = Some((cwd.to_path_buf(), query.clone()));
        if self.explorer_search_last == last {
            return;
        }
        // Another folder's hits do not belong here while its search runs.
        if self
            .explorer_search_last
            .as_ref()
            .is_some_and(|(root, _)| root != cwd)
        {
            self.explorer_search.clear();
            self.explorer_search_error = None;
        }
        self.explorer_search_last = last;
        self.explorer_search_generation = self.explorer_search_generation.wrapping_add(1);
        if query.is_empty() {
            self.explorer_search.clear();
            self.explorer_search_error = None;
            self.explorer_search_pending = false;
            return;
        }
        let id = self.explorer_search_generation;
        let show_ignored = self.preferences.show_ignored;
        let root = cwd.to_path_buf();
        // A refused search answers with its error; nothing is searching.
        self.explorer_search_pending = (self.jobs)
            .send(Job::Search {
                id,
                root,
                query,
                show_ignored,
            })
            .is_ok();
    }
}

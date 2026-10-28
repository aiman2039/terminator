use super::*;

impl LspManager {
    /// Ask for hover at a UTF-16 position; the answer lands in
    /// [`hover_popup`]. Without a server the popup explains why.
    pub fn request_hover(&mut self, path: &Path, position: L::Position, cursor: (usize, usize)) {
        let path = path.to_owned();
        if let Some(note) = self.server_status(&path) {
            self.hover = Some(HoverPopup {
                path,
                cursor_line: cursor.0,
                cursor_col: cursor.1,
                text: note,
                markdown: false,
                open: true,
                status: true,
            });
            return;
        }
        let key = self.docs.get(&path).map(|doc| doc.server.clone());
        let uri = self.docs.get(&path).map(|doc| doc.uri.clone());
        let (Some(key), Some(uri)) = (key, uri) else {
            return;
        };
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready {
            self.hover = Some(HoverPopup {
                path,
                cursor_line: cursor.0,
                cursor_col: cursor.1,
                text: "Language server is starting…".into(),
                markdown: false,
                open: true,
                status: true,
            });
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::hover(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(
            number,
            Pending::Hover {
                path,
                cursor: (cursor.0, cursor.1),
            },
        );
    }

    /// Ask for the definition of the symbol at a UTF-16 position; the
    /// answer lands in [`take_goto`]. Silent without a ready server
    /// (nothing to jump to, same as an empty answer).
    pub fn request_definition(&mut self, path: &Path, position: L::Position) {
        let path = path.to_owned();
        let key = self.docs.get(&path).map(|doc| doc.server.clone());
        let uri = self.docs.get(&path).map(|doc| doc.uri.clone());
        let (Some(key), Some(uri)) = (key, uri) else {
            return;
        };
        let Some(server) = self.servers.get_mut(&key) else {
            return;
        };
        if !server.ready {
            return;
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        let _ = server
            .inbox
            .send(L::definition(&L::RequestId::Number(number), &uri, position));
        server.pending.insert(number, Pending::Definition { path });
    }

    /// Ask for references at a UTF-16 position. The response is available
    /// through [`take_references`] and never changes files.
    pub fn request_references(&mut self, path: &Path, position: L::Position) {
        let Some(doc) = self.docs.get(path) else {
            return;
        };
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let Some(server) = self.servers.get_mut(&key).filter(|server| server.ready) else {
            return;
        };
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return;
        };
        if server
            .inbox
            .send(L::references(&L::RequestId::Number(number), &uri, position))
            .is_ok()
        {
            server.pending.insert(
                number,
                Pending::References {
                    path: path.to_owned(),
                },
            );
        }
    }

    /// Start a call / type hierarchy at a UTF-16 position. Returns the
    /// request generation (answers for older ones are stale), or why the
    /// server cannot answer.
    pub fn request_hierarchy(
        &mut self,
        path: &Path,
        position: L::Position,
        kind: L::HierarchyKind,
    ) -> Result<u64, String> {
        if let Some(note) = self.server_status(path) {
            return Err(note);
        }
        let doc = self
            .docs
            .get(path)
            .ok_or("No language server tracks this file")?;
        let (key, uri) = (doc.server.clone(), doc.uri.clone());
        let server = self
            .servers
            .get_mut(&key)
            .filter(|server| server.ready)
            .ok_or("Language server is not ready")?;
        if !L::supports_hierarchy(&server.capabilities, kind) {
            return Err(format!(
                "The {} language server has no {} hierarchy",
                key.language,
                match kind {
                    L::HierarchyKind::Call => "call",
                    L::HierarchyKind::Type => "type",
                }
            ));
        }
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return Err("Language server request ids exhausted".into());
        };
        self.hierarchy_generation = self.hierarchy_generation.wrapping_add(1);
        let generation = self.hierarchy_generation;
        server
            .inbox
            .send(L::prepare_hierarchy(L::PrepareHierarchyInput {
                id: &L::RequestId::Number(number),
                uri: &uri,
                position,
                kind,
            }))
            .map_err(|_| "Language server stopped".to_owned())?;
        server.pending.insert(
            number,
            Pending::Hierarchy {
                generation,
                node: None,
            },
        );
        Ok(generation)
    }

    /// A fresh hierarchy generation (a direction switch drops answers
    /// still in flight for the old one).
    pub fn next_hierarchy_generation(&mut self) -> u64 {
        self.hierarchy_generation = self.hierarchy_generation.wrapping_add(1);
        self.hierarchy_generation
    }

    /// Ask for one hierarchy node's children (through the server that
    /// serves `path`). False when no ready server can take it.
    pub fn request_hierarchy_children(&mut self, input: RequestHierarchyChildrenInput<'_>) -> bool {
        let RequestHierarchyChildrenInput {
            path,
            item,
            kind,
            direction,
            generation,
            node,
        } = input;
        let Some(key) = self.docs.get(path).map(|doc| doc.server.clone()) else {
            return false;
        };
        let Some(server) = self.servers.get_mut(&key).filter(|server| server.ready) else {
            return false;
        };
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return false;
        };
        if server
            .inbox
            .send(L::hierarchy_children(L::HierarchyChildrenInput {
                id: &L::RequestId::Number(number),
                item,
                kind,
                direction,
            }))
            .is_err()
        {
            return false;
        }
        server.pending.insert(
            number,
            Pending::Hierarchy {
                generation,
                node: Some(node),
            },
        );
        true
    }

    /// Request a preview of a symbol rename. No file is changed here.
    pub fn request_rename(
        &mut self,
        path: &Path,
        position: L::Position,
        new_name: &str,
    ) -> Result<(), String> {
        if let Some(note) = self.server_status(path) {
            return Err(note);
        }
        let doc = self
            .docs
            .get(path)
            .ok_or("No language server tracks this file")?;
        let (key, uri, revision) = (doc.server.clone(), doc.uri.clone(), doc.submitted);
        let server = self
            .servers
            .get_mut(&key)
            .filter(|server| server.ready)
            .ok_or("Language server is not ready")?;
        let L::RequestId::Number(number) = server.ids.next_id() else {
            return Err("Could not allocate a language server request".into());
        };
        server
            .inbox
            .send(L::rename(L::RenameInput {
                id: &L::RequestId::Number(number),
                uri: &uri,
                position,
                new_name,
            }))
            .map_err(|_| "Language server connection closed".to_string())?;
        server.pending.insert(
            number,
            Pending::Rename {
                path: path.to_owned(),
                new_name: new_name.to_owned(),
                revision,
            },
        );
        Ok(())
    }

    /// Current hover popup for `path`, dismissed when the cursor moved
    /// on. Returns the text to show, if any.
    pub fn hover_popup(&mut self, path: &Path, cursor: (usize, usize)) -> Option<(String, bool)> {
        let popup = self.hover.as_mut()?;
        if popup.path != path || (popup.cursor_line, popup.cursor_col) != cursor || !popup.open {
            if popup.path == path {
                self.hover = None;
            }
            return None;
        }
        Some((popup.text.clone(), popup.markdown))
    }

    pub fn close_hover(&mut self) {
        if let Some(popup) = self.hover.as_mut() {
            popup.open = false;
        }
    }

    /// Finished goto jump, if any.
    pub fn take_goto(&mut self) -> Option<GotoJump> {
        self.goto.take()
    }
}

//! Settings → Languages: user-defined language servers. Each row is
//! file types, a command and its arguments. They are saved in the same
//! settings map as the server paths (see `lsp::store_custom_servers`).
//! A user server never starts before the project trust answer.
use crate::*;
use terminator_native_edit::lsp::{self as L, CustomServer};

/// Search words for the user-server rows (section haystack).
pub(in crate::settings_ui) const CUSTOM_SERVER_KEYWORDS: &str =
    " user-defined custom language servers file types extensions command arguments add remove";

/// The add form's text, kept in egui memory while Settings is open.
#[derive(Clone, Default)]
struct AddForm {
    name: String,
    types: String,
    command: String,
    args: String,
    error: Option<String>,
}

/// A new server from the add form, or why it cannot be added.
fn server_from_form(form: &AddForm) -> Result<CustomServer, String> {
    let name = form.name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    let extensions: Vec<String> = form
        .types
        .split([',', ' '])
        .map(|ext| ext.trim().trim_start_matches('.').to_owned())
        .filter(|ext| !ext.is_empty())
        .collect();
    if extensions.is_empty() {
        return Err("Enter one or more file types, for example: zig, zon".into());
    }
    let command = form.command.trim();
    if command.is_empty() {
        return Err("Enter the server command.".into());
    }
    let args = shlex::split(form.args.trim()).ok_or("Arguments have an open quote.")?;
    let server = CustomServer {
        name: name.to_owned(),
        extensions,
        command: command.to_owned(),
        args,
    };
    if L::custom_languages(&server).is_empty() {
        return Err("No known file type in the list (plain text cannot take a server).".into());
    }
    Ok(server)
}

impl App {
    pub(super) fn custom_server_rows(&mut self, ui: &mut egui::Ui) {
        if !self.field_visible("User-defined language servers", CUSTOM_SERVER_KEYWORDS) {
            return;
        }
        settings_controls::sub_heading(ui, "User-defined language servers");
        settings_controls::description_line(
            ui,
            "A server here wins over the built-in one for its file types. It starts only in \
             projects you trust. Nothing is installed for you.",
        );
        ui.add_space(6.0);
        let mut servers = L::parse_custom_servers(&self.settings_draft.lsp_servers);
        let mut remove = None;
        for (index, server) in servers.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.button("Remove").clicked() {
                    remove = Some(index);
                }
                ui.strong(&server.name);
                ui.label(server.extensions.join(", "));
                ui.weak(format!("{} {}", server.command, shlex_join(&server.args)));
                if self.settings_executable(server.command.trim()).is_none() {
                    ui.colored_label(
                        appearance::color(&self.theme.status_failed),
                        "Language server not found",
                    );
                }
            });
        }
        if let Some(index) = remove {
            servers.remove(index);
            L::store_custom_servers(&mut self.settings_draft.lsp_servers, &servers);
        }
        if let Some(server) = self.custom_server_form(ui) {
            servers.retain(|old| old.name != server.name);
            servers.push(server);
            L::store_custom_servers(&mut self.settings_draft.lsp_servers, &servers);
        }
        ui.add_space(10.0);
    }

    /// The add form. Returns a server when Add was clicked with valid text.
    fn custom_server_form(&self, ui: &mut egui::Ui) -> Option<CustomServer> {
        let id = egui::Id::unique("settings-custom-lsp-form");
        let mut form: AddForm = ui.data(|data| data.get_temp(id)).unwrap_or_default();
        let mut added = None;
        egui::Grid::new("settings-custom-lsp-grid")
            .num_columns(2)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                for (label, value, hint) in [
                    ("Name", &mut form.name, "zls"),
                    ("File types", &mut form.types, "zig, zon"),
                    ("Command", &mut form.command, "zls"),
                    ("Arguments", &mut form.args, "--stdio"),
                ] {
                    ui.label(label);
                    ui.add(
                        appearance::singleline(value)
                            .hint_text(hint)
                            .desired_width(260.0),
                    );
                    ui.end_row();
                }
            });
        ui.horizontal(|ui| {
            if ui.button("Add server").clicked() {
                match server_from_form(&form) {
                    Ok(server) => {
                        added = Some(server);
                        form = AddForm::default();
                    }
                    Err(error) => form.error = Some(error),
                }
            }
            if let Some(error) = &form.error {
                ui.colored_label(appearance::color(&self.theme.status_failed), error);
            }
        });
        ui.data_mut(|data| data.insert_temp(id, form));
        added
    }
}

fn shlex_join(args: &[String]) -> String {
    shlex::try_join(args.iter().map(String::as_str)).unwrap_or_else(|_| args.join(" "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(types: &str, args: &str) -> AddForm {
        AddForm {
            name: "kotlin".into(),
            types: types.into(),
            command: "kotlin-language-server".into(),
            args: args.into(),
            error: None,
        }
    }

    #[test]
    fn add_form_splits_file_types_and_quoted_arguments() {
        let server = server_from_form(&form(".kt, kts", "--log 'a b'")).expect("valid");
        assert_eq!(server.extensions, ["kt", "kts"]);
        assert_eq!(server.args, ["--log", "a b"]);
    }

    #[test]
    fn add_form_refuses_missing_fields_and_plain_text_types() {
        assert!(server_from_form(&form("", "")).is_err());
        assert!(server_from_form(&form("no-such-type-qq", "")).is_err());
        assert!(server_from_form(&form("kt", "'open")).is_err());
        let unnamed = AddForm {
            name: " ".into(),
            ..form("kt", "")
        };
        assert!(server_from_form(&unnamed).is_err());
    }
}

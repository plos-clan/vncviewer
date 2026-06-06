use keyring_core::{Entry, Error as KeyringError};
use vnc::Credentials;

use crate::egui::{self, TextEdit, Ui};

const CONNECT_FORM_WIDTH: f32 = 360.0;
const CREDENTIAL_SERVICE: &str = "vncviewer.connect_form";

pub(crate) struct ConnectForm {
    address: String,
    port: String,
    username: String,
    password: String,
}

impl Default for ConnectForm {
    fn default() -> Self {
        let mut form = Self {
            address: "192.168.0.1".to_owned(),
            port: "5900".to_owned(),
            username: String::new(),
            password: String::new(),
        };

        for (key, value) in [
            ("address", &mut form.address),
            ("port", &mut form.port),
            ("username", &mut form.username),
            ("password", &mut form.password),
        ] {
            match Entry::new(CREDENTIAL_SERVICE, key).and_then(|entry| entry.get_password()) {
                Ok(secret) => *value = secret,
                Err(KeyringError::NoEntry | KeyringError::NoDefaultStore) => {}
                Err(e) => tracing::warn!("Failed to load saved {key}: {e}"),
            }
        }

        form
    }
}

impl ConnectForm {
    fn connect_request(&self) -> Option<(String, Credentials)> {
        let address = self.address.trim();
        let port = self.port.trim();
        if address.is_empty() || port.is_empty() {
            return None;
        }

        for (key, secret) in [
            ("address", address),
            ("port", port),
            ("username", self.username.trim()),
            ("password", self.password.as_str()),
        ] {
            let result = Entry::new(CREDENTIAL_SERVICE, key).and_then(|entry| {
                if secret.is_empty() {
                    entry.delete_credential()
                } else {
                    entry.set_password(secret)
                }
            });

            match result {
                Ok(()) => {}
                Err(KeyringError::NoEntry) if secret.is_empty() => {}
                Err(KeyringError::NoDefaultStore) => {}
                Err(e) => tracing::warn!("Failed to save {key}: {e}"),
            }
        }

        let username = self.username.trim();
        Some((
            format!("{address}:{port}"),
            Credentials::new(
                (!username.is_empty()).then(|| username.to_owned()),
                (!self.password.is_empty()).then(|| self.password.clone()),
            ),
        ))
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui) -> Option<(String, Credentials)> {
        let mut connect = false;

        ui.centered_and_justified(|ui| {
            ui.vertical(|ui| {
                ui.set_width(CONNECT_FORM_WIDTH);
                ui.label("Address");
                ui.horizontal(|ui| {
                    let port_width = 82.0;
                    let spacing = ui.spacing().item_spacing.x;
                    let height = ui.spacing().interact_size.y;
                    ui.add_sized(
                        [ui.available_width() - port_width - spacing, height],
                        TextEdit::singleline(&mut self.address),
                    );
                    ui.add_sized(
                        [port_width, height],
                        TextEdit::singleline(&mut self.port).hint_text("Port"),
                    );
                    self.port.retain(|c| c.is_ascii_digit());
                });
                ui.add_space(6.0);

                ui.label("Username");
                ui.text_edit_singleline(&mut self.username);
                ui.add_space(6.0);

                ui.label("Password");
                ui.add(TextEdit::singleline(&mut self.password).password(true));
                ui.add_space(10.0);

                let can_connect = !self.address.trim().is_empty() && !self.port.trim().is_empty();
                let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));

                let button_response = ui.add_enabled_ui(can_connect, |ui| {
                    ui.add_sized([ui.available_width(), 28.0], egui::Button::new("Connect"))
                });

                connect = button_response.inner.clicked() || (can_connect && enter_pressed)
            });
        });

        connect.then(|| self.connect_request()).flatten()
    }
}

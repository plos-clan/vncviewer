use keyring_core::{Entry, Error as KeyringError};
use serde::{Deserialize, Serialize};
use vnc::Credentials;

use crate::egui::{self, TextEdit, Ui};

const CONNECT_FORM_WIDTH: f32 = 300.0;
const CREDENTIAL_SERVICE: &str = "vncviewer.connect_form";

#[derive(Clone, Default, Deserialize, Serialize)]
struct ConnectionProfile {
    name: String,
    address: String,
    port: String,
    username: String,
    password: String,
}

impl ConnectionProfile {
    fn default_name(&self) -> String {
        let address = self.address.trim();
        let port = self.port.trim();
        let username = self.username.trim();
        let host = match (address.is_empty(), port.is_empty()) {
            (true, _) => String::new(),
            (_, true) => address.to_owned(),
            _ => format!("{address}:{port}"),
        };

        match (host.is_empty(), username.is_empty()) {
            (true, true) => String::new(),
            (true, false) => username.to_owned(),
            (false, true) => host,
            (false, false) => format!("{host} ({username})"),
        }
    }
}

pub(crate) struct ConnectForm {
    profiles: Vec<ConnectionProfile>,
    selected: Option<usize>,
    current: ConnectionProfile,
}

impl Default for ConnectForm {
    fn default() -> Self {
        let loaded_profiles = match Entry::new(CREDENTIAL_SERVICE, "address_book")
            .and_then(|entry| entry.get_password())
        {
            Ok(secret) => Some(serde_json::from_str(&secret).unwrap_or_else(|e| {
                tracing::warn!("Failed to load address book: {e}");
                Vec::new()
            })),
            Err(KeyringError::NoEntry | KeyringError::NoDefaultStore) => None,
            Err(e) => {
                tracing::warn!("Failed to load address book: {e}");
                None
            }
        };

        let had_address_book = loaded_profiles.is_some();
        let profiles = loaded_profiles.unwrap_or_default();
        let selected = (!profiles.is_empty()).then_some(0);
        let mut current = profiles.first().cloned().unwrap_or(ConnectionProfile {
            address: "192.168.0.1".to_owned(),
            port: "5900".to_owned(),
            ..Default::default()
        });

        if !had_address_book {
            for (key, value) in [
                ("address", &mut current.address),
                ("port", &mut current.port),
                ("username", &mut current.username),
                ("password", &mut current.password),
            ] {
                let entry = Entry::new(CREDENTIAL_SERVICE, key);
                match entry.and_then(|entry| entry.get_password()) {
                    Ok(secret) => *value = secret,
                    Err(KeyringError::NoEntry | KeyringError::NoDefaultStore) => {}
                    Err(e) => tracing::warn!("Failed to load saved {key}: {e}"),
                }
            }
        }

        Self {
            profiles,
            selected,
            current,
        }
    }
}

impl ConnectForm {
    fn save_profiles(&self) {
        let secret = match serde_json::to_string(&self.profiles) {
            Ok(secret) => secret,
            Err(e) => {
                tracing::warn!("Failed to save address book: {e}");
                return;
            }
        };

        match Entry::new(CREDENTIAL_SERVICE, "address_book")
            .and_then(|entry| entry.set_password(&secret))
        {
            Ok(()) | Err(KeyringError::NoDefaultStore) => {}
            Err(e) => tracing::warn!("Failed to save address book: {e}"),
        }
    }

    fn save_current(&mut self) {
        let profile = ConnectionProfile {
            name: self.current.name.trim().to_owned(),
            address: self.current.address.trim().to_owned(),
            port: self.current.port.trim().to_owned(),
            username: self.current.username.trim().to_owned(),
            password: self.current.password.clone(),
        };

        if let Some(index) = self
            .selected
            .take()
            .filter(|&index| index < self.profiles.len())
        {
            self.profiles.remove(index);
        }

        self.profiles.retain(|entry| {
            entry.address != profile.address
                || entry.port != profile.port
                || entry.username != profile.username
        });
        self.profiles.insert(0, profile.clone());
        self.current = profile;
        self.selected = Some(0);
        self.save_profiles();
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui) -> Option<(String, Credentials)> {
        let profile_name = |profile: &ConnectionProfile| {
            let name = profile.name.trim();
            if name.is_empty() {
                profile.default_name()
            } else {
                name.to_owned()
            }
        };

        let pick_profile = |ui: &mut Ui, form: &Self, width| {
            let mut selected = form.selected;
            let selected_text = selected
                .and_then(|index| form.profiles.get(index))
                .map(profile_name)
                .unwrap_or_else(|| "New connection".to_owned());

            egui::ComboBox::from_id_salt("address_book")
                .selected_text(selected_text)
                .width(width)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut selected, None, "New connection");
                    for (index, profile) in form.profiles.iter().enumerate() {
                        let name = profile_name(profile);
                        ui.selectable_value(&mut selected, Some(index), name);
                    }
                });

            (selected != form.selected).then_some(selected)
        };

        let address_book = |ui: &mut Ui, form: &mut Self| {
            ui.label("Address Book");
            let delete_width = ui.spacing().interact_size.y;
            let (picked, delete) = ui
                .horizontal(|ui| {
                    let spacing = ui.spacing().item_spacing.x;
                    let width = ui.available_width() - delete_width - spacing;
                    let picked = pick_profile(ui, form, width);
                    let delete_size = egui::Vec2::splat(delete_width);
                    let button = egui::Button::new("\u{00d7}").min_size(delete_size);
                    let delete = ui
                        .add_enabled(form.selected.is_some(), button)
                        .on_hover_text("Delete");

                    (picked, delete.clicked())
                })
                .inner;

            if let Some(picked) = picked {
                form.selected = picked.filter(|&index| index < form.profiles.len());
                let profile = form.selected.and_then(|index| form.profiles.get(index));
                if let Some(profile) = profile {
                    form.current = profile.clone();
                } else {
                    form.current.name.clear();
                }
            }

            if delete
                && let Some(index) = form
                    .selected
                    .take()
                    .filter(|&index| index < form.profiles.len())
            {
                form.profiles.remove(index);
                form.current.name.clear();
                form.save_profiles();
            }

            ui.add_space(4.0);
        };

        let profile_fields = |ui: &mut Ui, profile: &mut ConnectionProfile| {
            let field_width = ui.available_width();
            let height = ui.spacing().interact_size.y;
            let field = |ui: &mut Ui, label, edit| {
                ui.label(label);
                ui.add_sized([field_width, height], edit);
                ui.add_space(4.0);
            };

            let name_hint = profile.default_name();
            field(
                ui,
                "Name",
                TextEdit::singleline(&mut profile.name).hint_text(name_hint),
            );

            let port_width = 82.0;
            let spacing = ui.spacing().item_spacing.x;
            let address_width = field_width - port_width - spacing;

            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(address_width);
                    ui.label("Address");
                    ui.add_sized(
                        [address_width, height],
                        TextEdit::singleline(&mut profile.address),
                    );
                });
                ui.vertical(|ui| {
                    ui.set_width(port_width);
                    ui.label("Port");
                    ui.add_sized(
                        [port_width, height],
                        TextEdit::singleline(&mut profile.port).hint_text("Port"),
                    );
                });
                profile.port.retain(|c| c.is_ascii_digit());
            });
            ui.add_space(4.0);

            field(ui, "Username", TextEdit::singleline(&mut profile.username));
            field(
                ui,
                "Password",
                TextEdit::singleline(&mut profile.password).password(true),
            );
            ui.add_space(4.0);
        };

        let submit_button = |ui: &mut Ui, form: &mut Self| {
            let has_address = !form.current.address.trim().is_empty();
            let has_port = !form.current.port.trim().is_empty();
            let can_connect = has_address && has_port;
            let enter_pressed = ui.input(|input| input.key_pressed(egui::Key::Enter));
            let height = ui.spacing().interact_size.y + 4.0;
            let original_name = form
                .selected
                .and_then(|index| form.profiles.get(index))
                .map(|profile| profile.name.trim())
                .unwrap_or_default();
            let rename_pending = form.current.name.trim() != original_name;

            let button_response = ui.add_enabled_ui(can_connect, |ui| {
                ui.add_sized(
                    [ui.available_width(), height],
                    egui::Button::new(if rename_pending { "Save" } else { "Connect" }),
                )
            });

            let clicked = button_response.inner.clicked();
            let submit = clicked || (can_connect && enter_pressed);
            if !submit {
                return None;
            }

            form.save_current();
            (!rename_pending).then(|| {
                let has_username = !form.current.username.is_empty();
                let has_password = !form.current.password.is_empty();
                let username = has_username.then(|| form.current.username.clone());
                let password = has_password.then(|| form.current.password.clone());

                (
                    format!("{}:{}", form.current.address, form.current.port),
                    Credentials::new(username, password),
                )
            })
        };

        ui.vertical(|ui| {
            ui.set_width(CONNECT_FORM_WIDTH);
            address_book(ui, self);
            profile_fields(ui, &mut self.current);
            submit_button(ui, self)
        })
        .inner
    }
}

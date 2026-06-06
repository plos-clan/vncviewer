use arboard::Clipboard;
use miniquad::{KeyCode, window};
use tokio::sync::mpsc::{Receiver, Sender};
use vnc::{ClientKeyEvent, ClientMouseEvent, Screen as VncScreen, VncEvent, X11Event};

use crate::egui::{self, ColorImage, Ui};
#[cfg(windows)]
use crate::platform::windows;

use super::viewer::VncViewer;

const VIEWER_WINDOW_SIZE: (u32, u32) = (1200, 1024);

#[derive(Default)]
struct Fullscreen {
    on: bool,
    #[cfg(windows)]
    windowed: Option<windows::WindowState>,
}

impl Fullscreen {
    #[cfg(windows)]
    fn set(&mut self) {
        let fullscreen = windows::WindowState::fullscreen;
        self.windowed = self.on.then(fullscreen).flatten();
    }
}

pub(crate) struct VncSession {
    input_tx: Sender<X11Event>,
    vnc_event_rx: Receiver<VncEvent>,
    remote_size: egui::Vec2,
    desktop_texture: Option<egui::TextureHandle>,
    error: Option<String>,
    latest_mouse_event: ClientMouseEvent,
    pub(crate) f8_menu_pos: egui::Pos2,
    resized_window: bool,
    fullscreen: Fullscreen,
    #[cfg(windows)]
    _keyboard_grab: windows::KeyboardGrab,
}

impl VncSession {
    pub(crate) fn new(input_tx: Sender<X11Event>, vnc_event_rx: Receiver<VncEvent>) -> Self {
        Self {
            input_tx,
            vnc_event_rx,
            remote_size: egui::Vec2::ZERO,
            desktop_texture: None,
            error: None,
            latest_mouse_event: Default::default(),
            f8_menu_pos: egui::pos2(0.0, 0.0),
            resized_window: false,
            fullscreen: Fullscreen::default(),
            #[cfg(windows)]
            _keyboard_grab: windows::KeyboardGrab::install(),
        }
    }

    pub(crate) fn ui(&mut self, ui: &mut Ui) -> bool {
        let mut disconnect = false;

        if let Some(error) = &self.error {
            ui.vertical_centered(|ui| {
                ui.add_space((ui.available_height() - 120.0).max(0.0) / 2.0);
                ui.heading("Connection failed");
                ui.add_space(8.0);
                ui.set_max_width((ui.available_width() - 32.0).min(420.0));
                ui.add(egui::Label::new(egui::RichText::new(error).weak()).wrap());
                ui.add_space(12.0);

                disconnect = ui.button("Back").clicked();
            });

            return disconnect;
        }

        if let Some(texture) = &self.desktop_texture {
            let vnc_widget = VncViewer {
                texture,
                remote_size: self.remote_size,
                input_tx: &self.input_tx,
                latest_mouse_event: &mut self.latest_mouse_event,
            };
            ui.add(vnc_widget);
        }

        disconnect || self.context_menu(ui)
    }

    fn context_menu(&mut self, ui: &mut Ui) -> bool {
        let mut disconnect = false;

        egui::Popup::new(
            egui::Id::new("vnc_f8_menu"),
            ui.ctx().clone(),
            egui::PopupAnchor::Position(self.f8_menu_pos),
            ui.layer_id(),
        )
        .kind(egui::PopupKind::Menu)
        .layout(egui::Layout::top_down_justified(egui::Align::Min))
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .open_memory(None)
        .show(|ui| {
            ui.set_min_width(150.0);

            if ui.button("Disconnect").clicked() {
                disconnect = true;
                ui.close();
            }

            if ui.checkbox(&mut self.fullscreen.on, "Fullscreen").changed() {
                self.fullscreen.set();
                ui.close();
            }

            if ui.button("Send F8").clicked() {
                for down in [true, false] {
                    let event = ClientKeyEvent {
                        keycode: KeyCode::F8 as u32,
                        down,
                    };
                    self.send_input(X11Event::KeyEvent(event), "F8 key");
                }
                ui.close();
            }

            if ui.button("Send Ctrl+Alt+Del").clicked() {
                for (keycode, down) in [
                    (KeyCode::LeftControl as u32, true),
                    (KeyCode::LeftAlt as u32, true),
                    (KeyCode::Delete as u32, true),
                    (KeyCode::Delete as u32, false),
                    (KeyCode::LeftAlt as u32, false),
                    (KeyCode::LeftControl as u32, false),
                ] {
                    let event = ClientKeyEvent { keycode, down };
                    self.send_input(X11Event::KeyEvent(event), "Ctrl+Alt+Del key");
                }
                ui.close();
            }
        });

        disconnect
    }

    pub(crate) fn handle_events(&mut self, ctx: &egui::Context, clipboard: &mut Option<Clipboard>) {
        while let Ok(event) = self.vnc_event_rx.try_recv() {
            match event {
                VncEvent::SetResolution(screen) => {
                    let VncScreen { width, height } = screen;
                    self.remote_size = egui::vec2(width as f32, height as f32);

                    self.desktop_texture = Some(ctx.load_texture(
                        "vnc_desktop",
                        ColorImage::filled([width as usize, height as usize], egui::Color32::BLACK),
                        egui::TextureOptions::LINEAR,
                    ));
                }
                VncEvent::RawImage(rect, data) => {
                    if let Some(texture) = &mut self.desktop_texture {
                        let image = ColorImage::from_rgba_unmultiplied(
                            [rect.width as usize, rect.height as usize],
                            &data,
                        );

                        texture.set_partial(
                            [rect.x as usize, rect.y as usize],
                            image,
                            egui::TextureOptions::LINEAR,
                        );

                        self.resize_window_once();
                    }
                }
                VncEvent::Text(text) => {
                    println!("Remote clipboard: {text}");

                    if let Some(clipboard) = clipboard {
                        if let Err(e) = clipboard.set_text(text) {
                            tracing::error!("Failed to set local clipboard: {}", e);
                        }
                    } else {
                        tracing::error!("Local clipboard is unavailable");
                    }
                }
                VncEvent::Error(error) => {
                    self.error = Some(error);
                }
                _ => {}
            }
        }
    }

    fn resize_window_once(&mut self) {
        if self.resized_window {
            return;
        }

        #[cfg(windows)]
        windows::set_window_resizable(true);

        window::set_window_size(VIEWER_WINDOW_SIZE.0, VIEWER_WINDOW_SIZE.1);
        self.resized_window = true;
    }

    pub(crate) fn send_input(&self, event: X11Event, label: &str) {
        if let Err(e) = self.input_tx.try_send(event) {
            tracing::error!("Failed to send {label} event: {}", e);
        }
    }
}

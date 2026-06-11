use std::time::Duration;

use anyhow::Result;
use arboard::Clipboard;
use miniquad::{EventHandler, KeyCode, KeyMods, MouseButton, window};
use tokio::sync::mpsc::{Receiver, Sender, channel};
use tokio::time::{MissedTickBehavior, interval};
use tokio::{net::TcpStream, runtime::Handle};
use vnc::{ClientKeyEvent, Credentials, PixelFormat, VncConnector, VncEvent, X11Event};

use crate::egui::{self, EguiMq};
#[cfg(windows)]
use crate::platform::windows;

use super::connect::ConnectForm;
use super::session::VncSession;

pub(crate) const CONNECT_WINDOW_SIZE: (u32, u32) = (480, 410);

pub(crate) struct Stage {
    egui_mq: EguiMq,
    rt_handle: Handle,
    connect_form: ConnectForm,
    vnc: Option<VncSession>,
    clipboard: Option<Clipboard>,
}

impl Stage {
    pub(crate) fn new(egui_mq: EguiMq, rt_handle: Handle) -> Self {
        Self {
            egui_mq,
            rt_handle,
            connect_form: ConnectForm::default(),
            vnc: None,
            clipboard: Clipboard::new().ok(),
        }
    }

    fn start_connection(&mut self, host: String, credentials: Credentials) {
        let (vnc_event_tx, vnc_event_rx) = channel::<VncEvent>(128);
        let (input_tx, input_rx) = channel::<X11Event>(128);

        self.rt_handle.spawn(async move {
            if let Err(e) = vnc_task(host, credentials, input_rx, &vnc_event_tx).await {
                tracing::error!("VNC connection failed: {}", e);
                let _ = vnc_event_tx.send(VncEvent::Error(e.to_string())).await;
            }
        });

        self.vnc = Some(VncSession::new(input_tx, vnc_event_rx));
    }

    fn handle_vnc_events(&mut self) {
        let ctx = self.egui_mq.ctx.clone();
        if let Some(vnc) = &mut self.vnc {
            vnc.handle_events(&ctx, &mut self.clipboard);
        }
    }

    fn send_key(&self, keycode: KeyCode, down: bool) {
        let miniquad_key_to_x11_keysym = |key| {
            let key_val = key as u32;
            if (KeyCode::A as u32..=KeyCode::Z as u32).contains(&key_val) {
                key_val + 0x020
            } else {
                key_val
            }
        };

        if let Some(vnc) = &self.vnc {
            let keycode = miniquad_key_to_x11_keysym(keycode);
            let event = ClientKeyEvent { keycode, down };
            vnc.send_input(X11Event::KeyEvent(event), "key");
        }
    }

    fn send_local_clipboard(&mut self) {
        let Some(vnc) = &self.vnc else {
            return;
        };

        match self.clipboard.as_mut().map(Clipboard::get_text) {
            Some(Ok(text)) => vnc.send_input(X11Event::CopyText(text), "clipboard"),
            Some(Err(e)) => tracing::error!("Failed to read local clipboard: {}", e),
            None => tracing::error!("Local clipboard is unavailable"),
        }
    }
}

async fn vnc_task(
    host: String,
    credentials: Credentials,
    mut input_rx: Receiver<X11Event>,
    vnc_event_tx: &Sender<VncEvent>,
) -> Result<()> {
    let tcp = TcpStream::connect(&host).await?;

    let vnc_client = VncConnector::new(tcp)
        .allow_shared(true)
        .set_credentials(credentials)
        .set_pixel_format(PixelFormat::rgba())
        .build()?
        .try_start()
        .await?
        .finish()?;

    let mut refresh_timer = interval(Duration::from_millis(16));
    refresh_timer.set_missed_tick_behavior(MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = refresh_timer.tick() => {
                vnc_client.input(X11Event::Refresh).await?;
            }

            input_event = input_rx.recv() => {
                let Some(input_event) = input_event else {
                    break;
                };
                vnc_client.input(input_event).await?;
            }

            result = vnc_client.poll_event() => {
                match result {
                    Ok(Some(event)) => {
                        vnc_event_tx.send(event).await?;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("{}", e.to_string());
                        break;
                    }
                }
            }
        }
    }

    Ok(())
}

impl EventHandler for Stage {
    fn update(&mut self) {}

    fn draw(&mut self) {
        self.handle_vnc_events();
        let mut request = None;
        let mut disconnect = false;

        self.egui_mq.run_ui(|ui| {
            if let Some(vnc) = &mut self.vnc {
                egui::CentralPanel::default()
                    .frame(egui::Frame::NONE.fill(egui::Color32::BLACK))
                    .show_inside(ui, |ui| disconnect = vnc.ui(ui));
            } else {
                request = egui::CentralPanel::default()
                    .show_inside(ui, |ui| self.connect_form.ui(ui))
                    .inner;
            }
        });

        if disconnect {
            self.vnc = None;
            #[cfg(windows)]
            windows::set_window_resizable(false);
            window::set_window_size(CONNECT_WINDOW_SIZE.0, CONNECT_WINDOW_SIZE.1);
        }

        if let Some((host, credentials)) = request {
            self.start_connection(host, credentials);
        }
    }

    fn window_restored_event(&mut self) {
        self.send_local_clipboard();
    }

    fn mouse_motion_event(&mut self, x: f32, y: f32) {
        self.egui_mq.mouse_motion_event(x, y);
    }

    fn mouse_wheel_event(&mut self, dx: f32, dy: f32) {
        self.egui_mq.mouse_wheel_event(dx, dy);
    }

    fn mouse_button_down_event(&mut self, mb: MouseButton, x: f32, y: f32) {
        self.egui_mq.mouse_button_event(mb, x, y, true);
    }

    fn mouse_button_up_event(&mut self, mb: MouseButton, x: f32, y: f32) {
        self.egui_mq.mouse_button_event(mb, x, y, false);
    }

    fn char_event(&mut self, character: char, _keymods: KeyMods, _repeat: bool) {
        self.egui_mq.char_event(character);
    }

    fn key_down_event(&mut self, keycode: KeyCode, keymods: KeyMods, _repeat: bool) {
        self.egui_mq.key_event(keycode, keymods, true);
        if keycode == KeyCode::F8 {
            let ctx = &self.egui_mq.ctx;
            let popup_id = egui::Id::new("vnc_f8_menu");
            let pos = ctx
                .pointer_latest_pos()
                .or_else(|| ctx.pointer_hover_pos())
                .unwrap_or_else(|| ctx.content_rect().center());

            if let Some(vnc) = &mut self.vnc {
                vnc.f8_menu_pos = pos;
                egui::Popup::open_id(ctx, popup_id);
                return;
            }
        }

        self.send_key(keycode, true);
    }

    fn key_up_event(&mut self, keycode: KeyCode, keymods: KeyMods) {
        self.egui_mq.key_event(keycode, keymods, false);
        if keycode == KeyCode::F8 && self.vnc.is_some() {
            return;
        }

        self.send_key(keycode, false);
    }
}

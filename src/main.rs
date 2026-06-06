#![allow(unsafe_op_in_unsafe_fn)]
#![cfg_attr(windows, windows_subsystem = "windows")]

mod client;
mod egui;
mod platform;

use crate::egui::EguiMq;
use client::{CONNECT_WINDOW_SIZE, Stage};
use miniquad::conf::Conf;
use miniquad::window;
use tracing::Level;

fn main() {
    let level = if cfg!(debug_assertions) {
        Level::TRACE
    } else {
        Level::INFO
    };

    let subscriber = tracing_subscriber::fmt()
        .pretty()
        .with_max_level(level)
        .finish();

    tracing::subscriber::set_global_default(subscriber)
        .expect("failed to setting default subscriber");

    #[cfg(windows)]
    match windows_native_keyring_store::Store::new() {
        Ok(store) => keyring_core::set_default_store(store),
        Err(e) => tracing::warn!("Failed to initialize Windows credential store: {e}"),
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    match zbus_secret_service_keyring_store::Store::new() {
        Ok(store) => keyring_core::set_default_store(store),
        Err(e) => tracing::warn!("Failed to initialize Secret Service credential store: {e}"),
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let rt_handle = rt.handle().clone();

    let conf = Conf {
        window_title: "VNC Viewer".into(),
        high_dpi: true,
        sample_count: 0,
        window_width: CONNECT_WINDOW_SIZE.0 as i32,
        window_height: CONNECT_WINDOW_SIZE.1 as i32,
        window_resizable: false,
        ..Default::default()
    };

    miniquad::start(conf, move || {
        let egui_mq = EguiMq::new(window::new_rendering_backend());
        egui_mq.ctx.set_zoom_factor(window::dpi_scale());
        egui_mq
            .ctx
            .options_mut(|options| options.zoom_with_keyboard = false);

        Box::new(Stage::new(egui_mq, rt_handle.clone()))
    });
}

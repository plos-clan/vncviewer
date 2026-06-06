use tokio::sync::mpsc::Sender;
use vnc::{ClientMouseEvent, X11Event};

use crate::egui::{self, Rect, Response, Ui, Widget};

pub(crate) struct VncViewer<'a> {
    pub(crate) texture: &'a egui::TextureHandle,
    pub(crate) remote_size: egui::Vec2,
    pub(crate) latest_mouse_event: &'a mut ClientMouseEvent,
    pub(crate) input_tx: &'a Sender<X11Event>,
}

impl<'a> VncViewer<'a> {
    fn calculate_button_mask(&self, input: &egui::InputState) -> u8 {
        let mut button_mask = 0;

        button_mask |= if input.pointer.primary_down() { 1 } else { 0 };
        button_mask |= if input.pointer.middle_down() { 2 } else { 0 };
        button_mask |= if input.pointer.secondary_down() { 4 } else { 0 };

        button_mask
    }

    fn calculate_image_rect(&self, outer_rect: Rect) -> Rect {
        let outer_size = outer_rect.size();
        let image_aspect_ratio = self.texture.aspect_ratio();

        let inner_size = if image_aspect_ratio > outer_rect.aspect_ratio() {
            egui::vec2(outer_size.x, outer_size.x / image_aspect_ratio)
        } else {
            egui::vec2(outer_size.y * image_aspect_ratio, outer_size.y)
        };

        Rect::from_center_size(outer_rect.center(), inner_size)
    }

    fn handle_mouse_input(&mut self, input: &egui::InputState, response: &Response) {
        if !response.hovered() && !response.dragged() {
            return;
        }

        let Some(mouse_pos) = input.pointer.hover_pos() else {
            return;
        };

        let image_rect = self.calculate_image_rect(response.rect);

        if !image_rect.contains(mouse_pos) {
            return;
        }

        let relative_pos = (
            (mouse_pos.x - image_rect.min.x) / image_rect.width(),
            (mouse_pos.y - image_rect.min.y) / image_rect.height(),
        );

        let remote_pos = (
            (relative_pos.0 * self.remote_size.x) as u16,
            (relative_pos.1 * self.remote_size.y) as u16,
        );

        let send_pointer_event = |buttons| {
            let mouse_event = ClientMouseEvent {
                position_x: remote_pos.0,
                position_y: remote_pos.1,
                buttons,
            };

            if let Err(e) = self.input_tx.try_send(X11Event::PointerEvent(mouse_event)) {
                tracing::error!("Failed to send mouse event: {}", e);
            }
        };

        let buttons = self.calculate_button_mask(input);
        let mouse_event = ClientMouseEvent {
            position_x: remote_pos.0,
            position_y: remote_pos.1,
            buttons,
        };

        if mouse_event != *self.latest_mouse_event {
            *self.latest_mouse_event = mouse_event.clone();
            send_pointer_event(buttons);
        }

        for event in &input.raw.events {
            if let egui::Event::MouseWheel { delta, .. } = event {
                if delta.y == 0.0 {
                    continue;
                }

                let scroll_button = if delta.y > 0.0 { 8 } else { 16 };
                send_pointer_event(buttons | scroll_button);
                send_pointer_event(buttons);
            }
        }
    }
}

impl<'a> Widget for VncViewer<'a> {
    fn ui(mut self, ui: &mut Ui) -> Response {
        let scene = egui::Sense::click_and_drag();
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), scene);

        let image_rect = self.calculate_image_rect(rect);
        egui::Image::new(self.texture).paint_at(ui, image_rect);
        ui.ctx().input(|i| self.handle_mouse_input(i, &response));

        response
    }
}

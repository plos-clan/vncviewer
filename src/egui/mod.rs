mod painter;

pub(crate) use egui::*;

pub(crate) struct EguiMq {
    pub(crate) ctx: egui::Context,
    input: egui::RawInput,
    mq_ctx: Box<dyn miniquad::RenderingBackend>,
    painter: painter::Painter,
}

impl EguiMq {
    pub(crate) fn new(mut mq_ctx: Box<dyn miniquad::RenderingBackend>) -> Self {
        let painter = painter::Painter::new(&mut *mq_ctx);

        Self {
            ctx: egui::Context::default(),
            input: egui::RawInput::default(),
            mq_ctx,
            painter,
        }
    }

    pub(crate) fn run_ui(&mut self, run_ui: impl FnMut(&mut egui::Ui)) {
        let screen_size = miniquad::window::screen_size();
        self.input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(screen_size.0, screen_size.1) / self.ctx.pixels_per_point(),
        ));
        self.input.time = Some(miniquad::date::now());

        let output = self.ctx.run_ui(self.input.take(), run_ui);
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        self.handle_platform_output(output.platform_output);
        self.painter.paint(
            &mut *self.mq_ctx,
            primitives,
            &output.textures_delta,
            &self.ctx,
        );
    }

    pub(crate) fn mouse_motion_event(&mut self, x: f32, y: f32) {
        self.input.events.push(egui::Event::PointerMoved(egui::pos2(
            x / self.ctx.pixels_per_point(),
            y / self.ctx.pixels_per_point(),
        )));
    }

    pub(crate) fn mouse_wheel_event(&mut self, dx: f32, dy: f32) {
        self.input.events.push(egui::Event::MouseWheel {
            modifiers: self.input.modifiers,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(dx, dy),
            phase: egui::TouchPhase::Move,
        });
    }

    pub(crate) fn char_event(&mut self, chr: char) {
        let private_use = ('\u{e000}'..='\u{f8ff}').contains(&chr)
            || ('\u{f0000}'..='\u{ffffd}').contains(&chr)
            || ('\u{100000}'..='\u{10fffd}').contains(&chr);
        if !private_use
            && !chr.is_ascii_control()
            && !self.input.modifiers.ctrl
            && !self.input.modifiers.mac_cmd
        {
            self.input.events.push(egui::Event::Text(chr.to_string()));
        }
    }

    pub(crate) fn key_event(
        &mut self,
        keycode: miniquad::KeyCode,
        keymods: miniquad::KeyMods,
        pressed: bool,
    ) {
        let modifiers = egui::Modifiers {
            alt: keymods.alt,
            ctrl: keymods.ctrl,
            shift: keymods.shift,
            mac_cmd: keymods.logo && cfg!(target_os = "macos"),
            command: if cfg!(target_os = "macos") {
                keymods.logo
            } else {
                keymods.ctrl
            },
        };
        self.input.modifiers = modifiers;

        if pressed && modifiers.command {
            match keycode {
                miniquad::KeyCode::X => self.input.events.push(egui::Event::Cut),
                miniquad::KeyCode::C => self.input.events.push(egui::Event::Copy),
                miniquad::KeyCode::V => {
                    if let Some(text) = miniquad::window::clipboard_get() {
                        self.input.events.push(egui::Event::Text(text));
                    }
                }
                _ => {}
            }
            return;
        }

        if let Some(key) = Self::egui_key(keycode) {
            self.input.events.push(egui::Event::Key {
                key,
                pressed,
                modifiers,
                repeat: false,
                physical_key: None,
            });
        }
    }

    fn handle_platform_output(&mut self, output: egui::PlatformOutput) {
        for command in output.commands {
            match command {
                egui::OutputCommand::CopyText(text) => miniquad::window::clipboard_set(&text),
                egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyImage(_) => {}
            }
        }

        if output.cursor_icon == egui::CursorIcon::None {
            miniquad::window::show_mouse(false);
            return;
        }

        miniquad::window::show_mouse(true);
        miniquad::window::set_mouse_cursor(match output.cursor_icon {
            egui::CursorIcon::Default => miniquad::CursorIcon::Default,
            egui::CursorIcon::PointingHand => miniquad::CursorIcon::Pointer,
            egui::CursorIcon::Text => miniquad::CursorIcon::Text,
            egui::CursorIcon::ResizeHorizontal => miniquad::CursorIcon::EWResize,
            egui::CursorIcon::ResizeVertical => miniquad::CursorIcon::NSResize,
            egui::CursorIcon::ResizeNeSw => miniquad::CursorIcon::NESWResize,
            egui::CursorIcon::ResizeNwSe => miniquad::CursorIcon::NWSEResize,
            egui::CursorIcon::Help => miniquad::CursorIcon::Help,
            egui::CursorIcon::Wait | egui::CursorIcon::Progress => miniquad::CursorIcon::Wait,
            egui::CursorIcon::Crosshair => miniquad::CursorIcon::Crosshair,
            egui::CursorIcon::Move | egui::CursorIcon::AllScroll => miniquad::CursorIcon::Move,
            egui::CursorIcon::NotAllowed => miniquad::CursorIcon::NotAllowed,
            _ => miniquad::CursorIcon::Default,
        });
    }

    pub(crate) fn mouse_button_event(
        &mut self,
        button: miniquad::MouseButton,
        x: f32,
        y: f32,
        pressed: bool,
    ) {
        self.input.events.push(egui::Event::PointerButton {
            pos: egui::pos2(
                x / self.ctx.pixels_per_point(),
                y / self.ctx.pixels_per_point(),
            ),
            button: match button {
                miniquad::MouseButton::Left => egui::PointerButton::Primary,
                miniquad::MouseButton::Right => egui::PointerButton::Secondary,
                miniquad::MouseButton::Middle => egui::PointerButton::Middle,
                miniquad::MouseButton::Unknown => egui::PointerButton::Primary,
            },
            pressed,
            modifiers: self.input.modifiers,
        });
    }

    fn egui_key(keycode: miniquad::KeyCode) -> Option<egui::Key> {
        let code = keycode as u16;
        if (0x20..=0x7e).contains(&code) {
            let mut buf = [0; 4];
            return egui::Key::from_name((code as u8 as char).encode_utf8(&mut buf));
        }

        Some(match keycode {
            miniquad::KeyCode::Down => egui::Key::ArrowDown,
            miniquad::KeyCode::Left => egui::Key::ArrowLeft,
            miniquad::KeyCode::Right => egui::Key::ArrowRight,
            miniquad::KeyCode::Up => egui::Key::ArrowUp,
            miniquad::KeyCode::Escape => egui::Key::Escape,
            miniquad::KeyCode::Tab => egui::Key::Tab,
            miniquad::KeyCode::Backspace => egui::Key::Backspace,
            miniquad::KeyCode::Enter => egui::Key::Enter,
            miniquad::KeyCode::Space => egui::Key::Space,
            miniquad::KeyCode::Insert => egui::Key::Insert,
            miniquad::KeyCode::Delete => egui::Key::Delete,
            miniquad::KeyCode::Home => egui::Key::Home,
            miniquad::KeyCode::End => egui::Key::End,
            miniquad::KeyCode::PageUp => egui::Key::PageUp,
            miniquad::KeyCode::PageDown => egui::Key::PageDown,
            _ => return None,
        })
    }
}

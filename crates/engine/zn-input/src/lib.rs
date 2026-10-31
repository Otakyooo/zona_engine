//! zn-input: ввод (клавиатура/мышь), геймпад, ремап.

use gilrs::{Axis, Button, GamepadId, Gilrs};
use glam::Vec3;
use std::collections::HashSet;
use winit::event::WindowEvent;
use winit::keyboard::KeyCode;

#[derive(Default)]
pub struct InputState {
    pub keys: HashSet<KeyCode>,
    pub mouse_delta: (f32, f32),
    pub cursor_captured: bool,
    pub gamepad_left: (f32, f32),
    pub gamepad_right: (f32, f32),
    pub gamepad_fire: bool,
    gilrs: Option<Gilrs>,
    gamepad_id: Option<GamepadId>,
}

impl InputState {
    pub fn new() -> Self {
        let mut s = Self::default();
        if let Ok(g) = Gilrs::new() {
            if let Some((id, _gp)) = g.gamepads().next() {
                s.gamepad_id = Some(id);
            }
            s.gilrs = Some(g);
        }
        s
    }
    pub fn key_down(&self, key: KeyCode) -> bool {
        self.keys.contains(&key)
    }
    pub fn set_key(&mut self, key: KeyCode, down: bool) {
        if down {
            self.keys.insert(key);
        } else {
            self.keys.remove(&key);
        }
    }
    pub fn reset_frame(&mut self) {
        self.mouse_delta = (0.0, 0.0);
    }
    pub fn handle_window_event(&mut self, event: &WindowEvent) {
        if let WindowEvent::KeyboardInput { event, .. } = event {
            self.handle_key_event(event);
        }
    }

    pub fn handle_key_event(&mut self, event: &winit::event::KeyEvent) {
        use winit::keyboard::PhysicalKey;
        if let PhysicalKey::Code(code) = event.physical_key {
            self.set_key(code, event.state == winit::event::ElementState::Pressed);
        }
    }
    pub fn add_mouse_delta(&mut self, dx: f32, dy: f32) {
        self.mouse_delta.0 += dx;
        self.mouse_delta.1 += dy;
    }
    pub fn update_gamepad(&mut self) {
        if let Some(g) = self.gilrs.as_mut() {
            while let Some(ev) = g.next_event() {
                let _ = ev;
            }
            if let Some(id) = self.gamepad_id {
                let gp = g.gamepad(id);
                self.gamepad_left = (
                    gp.axis_data(Axis::LeftStickX)
                        .map(|a| a.value())
                        .unwrap_or(0.0),
                    gp.axis_data(Axis::LeftStickY)
                        .map(|a| a.value())
                        .unwrap_or(0.0),
                );
                self.gamepad_right = (
                    gp.axis_data(Axis::RightStickX)
                        .map(|a| a.value())
                        .unwrap_or(0.0),
                    gp.axis_data(Axis::RightStickY)
                        .map(|a| a.value())
                        .unwrap_or(0.0),
                );
                self.gamepad_fire =
                    gp.is_pressed(Button::RightTrigger2) || gp.is_pressed(Button::South);
            }
        }
    }
    pub fn movement_vector(&self) -> Vec3 {
        let mut v = Vec3::ZERO;
        if self.key_down(KeyCode::KeyW) {
            v.z -= 1.0;
        }
        if self.key_down(KeyCode::KeyS) {
            v.z += 1.0;
        }
        if self.key_down(KeyCode::KeyA) {
            v.x -= 1.0;
        }
        if self.key_down(KeyCode::KeyD) {
            v.x += 1.0;
        }
        // gamepad
        v.x += self.gamepad_left.0;
        v.z -= self.gamepad_left.1;
        if v.length_squared() > 0.0 {
            // clamp
            if v.length() > 1.0 {
                v = v.normalize();
            }
        }
        v
    }
}

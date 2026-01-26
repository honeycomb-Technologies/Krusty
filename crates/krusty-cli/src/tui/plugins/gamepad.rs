//! Gamepad Input Handler
//!
//! Provides controller support for plugins using gilrs.
//! Supports automatic detection and hotplugging of controllers.

use gilrs::{Button, Event, EventType, Gilrs};
use std::collections::HashSet;

use super::libretro::JoypadButton;

/// Gamepad state and input handling
pub struct GamepadHandler {
    /// Gilrs instance for gamepad management
    gilrs: Option<Gilrs>,
    /// Currently pressed buttons (libretro button IDs)
    pressed_buttons: HashSet<u8>,
    /// Whether a controller is connected
    pub connected: bool,
    /// Controller name (if connected)
    pub controller_name: Option<String>,
}

impl Default for GamepadHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl GamepadHandler {
    /// Create a new gamepad handler
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(g) => {
                tracing::info!("Gamepad handler initialized");
                Some(g)
            }
            Err(e) => {
                tracing::warn!("Failed to initialize gamepad support: {}", e);
                None
            }
        };

        let mut handler = Self {
            gilrs,
            pressed_buttons: HashSet::new(),
            connected: false,
            controller_name: None,
        };

        // Check for already-connected controllers
        handler.check_connected();
        handler
    }

    /// Check for connected controllers
    fn check_connected(&mut self) {
        if let Some(ref gilrs) = self.gilrs {
            for (_id, gamepad) in gilrs.gamepads() {
                if gamepad.is_connected() {
                    self.connected = true;
                    self.controller_name = Some(gamepad.name().to_string());
                    tracing::info!("Controller connected: {}", gamepad.name());
                    return;
                }
            }
        }
        self.connected = false;
        self.controller_name = None;
    }

    /// Poll for gamepad events and update button states
    /// Returns true if any button state changed
    pub fn poll(&mut self) -> bool {
        let gilrs = match self.gilrs.as_mut() {
            Some(g) => g,
            None => return false,
        };

        let mut changed = false;
        let mut need_reconnect_check = false;

        while let Some(Event { id, event, .. }) = gilrs.next_event() {
            match event {
                EventType::ButtonPressed(button, _) => {
                    if let Some(joypad) = map_button(button) {
                        self.pressed_buttons.insert(joypad as u8);
                        changed = true;
                        tracing::trace!("Button pressed: {:?} -> {:?}", button, joypad);
                    }
                }
                EventType::ButtonReleased(button, _) => {
                    if let Some(joypad) = map_button(button) {
                        self.pressed_buttons.remove(&(joypad as u8));
                        changed = true;
                        tracing::trace!("Button released: {:?}", button);
                    }
                }
                EventType::Connected => {
                    let gamepad = gilrs.gamepad(id);
                    self.connected = true;
                    self.controller_name = Some(gamepad.name().to_string());
                    tracing::info!("Controller connected: {}", gamepad.name());
                    changed = true;
                }
                EventType::Disconnected => {
                    tracing::info!("Controller disconnected");
                    need_reconnect_check = true;
                    changed = true;
                }
                EventType::AxisChanged(axis, value, _) => {
                    // Handle D-pad via axis (some controllers report it this way)
                    handle_axis_as_dpad(axis, value, &mut self.pressed_buttons);
                }
                _ => {}
            }
        }

        // Check for remaining controllers after processing all events
        if need_reconnect_check {
            self.check_connected();
        }

        changed
    }

    /// Check if a button is currently pressed
    #[allow(dead_code)]
    pub fn is_pressed(&self, button: JoypadButton) -> bool {
        self.pressed_buttons.contains(&(button as u8))
    }

    /// Get all currently pressed buttons as libretro button IDs
    pub fn pressed_buttons(&self) -> impl Iterator<Item = u8> + '_ {
        self.pressed_buttons.iter().copied()
    }
}

/// Map gilrs button to libretro joypad button
fn map_button(button: Button) -> Option<JoypadButton> {
    match button {
        // Face buttons (Switch layout: B=bottom, A=right, Y=left, X=top)
        // Map to SNES layout which libretro uses
        Button::South => Some(JoypadButton::B), // Switch B -> libretro B
        Button::East => Some(JoypadButton::A),  // Switch A -> libretro A
        Button::West => Some(JoypadButton::Y),  // Switch Y -> libretro Y
        Button::North => Some(JoypadButton::X), // Switch X -> libretro X

        // D-pad
        Button::DPadUp => Some(JoypadButton::Up),
        Button::DPadDown => Some(JoypadButton::Down),
        Button::DPadLeft => Some(JoypadButton::Left),
        Button::DPadRight => Some(JoypadButton::Right),

        // Shoulder buttons
        Button::LeftTrigger => Some(JoypadButton::L),
        Button::RightTrigger => Some(JoypadButton::R),
        Button::LeftTrigger2 => Some(JoypadButton::L2),
        Button::RightTrigger2 => Some(JoypadButton::R2),

        // Start/Select (Plus/Minus on Switch)
        Button::Start => Some(JoypadButton::Start),
        Button::Select => Some(JoypadButton::Select),

        // Stick buttons
        Button::LeftThumb => Some(JoypadButton::L3),
        Button::RightThumb => Some(JoypadButton::R3),

        _ => None,
    }
}

/// Handle axis input as D-pad (for controllers that report D-pad as axis)
fn handle_axis_as_dpad(axis: gilrs::Axis, value: f32, pressed: &mut HashSet<u8>) {
    use gilrs::Axis;

    const THRESHOLD: f32 = 0.5;

    match axis {
        Axis::LeftStickX | Axis::DPadX => {
            // Clear both left/right first
            pressed.remove(&(JoypadButton::Left as u8));
            pressed.remove(&(JoypadButton::Right as u8));

            if value < -THRESHOLD {
                pressed.insert(JoypadButton::Left as u8);
            } else if value > THRESHOLD {
                pressed.insert(JoypadButton::Right as u8);
            }
        }
        Axis::LeftStickY | Axis::DPadY => {
            // Clear both up/down first
            pressed.remove(&(JoypadButton::Up as u8));
            pressed.remove(&(JoypadButton::Down as u8));

            // Y axis is typically inverted (up = negative)
            if value < -THRESHOLD {
                pressed.insert(JoypadButton::Up as u8);
            } else if value > THRESHOLD {
                pressed.insert(JoypadButton::Down as u8);
            }
        }
        _ => {}
    }
}

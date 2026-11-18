//! Manettes : lecture via `gilrs` (XInput, DirectInput, evdev…), traduite
//! en navigation pour le launcher et en boutons pour la console émulée.
//!
//! Correspondance (par nom de bouton, comme sur une manette Xbox) :
//!
//! | Manette              | Launcher       | Game Boy |
//! |----------------------|----------------|----------|
//! | Croix / stick gauche | Se déplacer    | Croix    |
//! | A (bas)              | Valider        | A        |
//! | B (droite)           | Retour         | B        |
//! | LB / RB              | Page préc./suiv.| —       |
//! | Start                | Valider        | Start    |
//! | Select               | —              | Select   |
//! | Select + Start       | —              | Quitter la partie |

use emu_core::Button;
use gilrs::{Axis, Button as Pad, EventType, Gilrs};

const STICK_THRESHOLD: f32 = 0.5;
/// Délai avant qu'une direction maintenue se répète, puis intervalle de répétition.
const REPEAT_DELAY: f64 = 0.35;
const REPEAT_INTERVAL: f64 = 0.08;

/// Actions de navigation déclenchées pendant cette image.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Nav {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub confirm: bool,
    pub back: bool,
    pub page_up: bool,
    pub page_down: bool,
}

/// Ce que les manettes ont fait pendant cette image.
#[derive(Debug, Default, Clone)]
pub struct PadFrame {
    pub nav: Nav,
    /// Boutons de console actuellement maintenus.
    pub held: Vec<Button>,
    /// Select + Start (ou le bouton central) : revenir à la bibliothèque.
    pub exit_game: bool,
    /// Au moins une manette est branchée.
    pub connected: bool,
}

/// État brut des boutons, toutes manettes confondues.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct Raw {
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    south: bool,
    east: bool,
    start: bool,
    select: bool,
    mode: bool,
    lb: bool,
    rb: bool,
}

/// Transforme l'état brut d'une image en actions, avec répétition des directions.
#[derive(Default)]
struct Translator {
    previous: Raw,
    /// Prochain instant de répétition pour haut, bas, gauche, droite.
    next_repeat: [f64; 4],
}

impl Translator {
    fn frame(&mut self, raw: Raw, now: f64, connected: bool) -> PadFrame {
        let prev = self.previous;
        let edge = |cur: bool, before: bool| cur && !before;

        let mut dirs = [false; 4];
        let current = [raw.up, raw.down, raw.left, raw.right];
        let before = [prev.up, prev.down, prev.left, prev.right];
        for i in 0..4 {
            if current[i] && !before[i] {
                dirs[i] = true;
                self.next_repeat[i] = now + REPEAT_DELAY;
            } else if current[i] && now >= self.next_repeat[i] {
                dirs[i] = true;
                self.next_repeat[i] = now + REPEAT_INTERVAL;
            }
        }

        let mut held = Vec::new();
        for (on, button) in [
            (raw.up, Button::Up),
            (raw.down, Button::Down),
            (raw.left, Button::Left),
            (raw.right, Button::Right),
            (raw.south, Button::A),
            (raw.east, Button::B),
            (raw.start, Button::Start),
            (raw.select, Button::Select),
        ] {
            if on {
                held.push(button);
            }
        }

        let combo = raw.select && raw.start;
        let frame = PadFrame {
            nav: Nav {
                up: dirs[0],
                down: dirs[1],
                left: dirs[2],
                right: dirs[3],
                confirm: edge(raw.south, prev.south)
                    || (edge(raw.start, prev.start) && !raw.select),
                back: edge(raw.east, prev.east),
                page_up: edge(raw.lb, prev.lb),
                page_down: edge(raw.rb, prev.rb),
            },
            held,
            exit_game: edge(combo, prev.select && prev.start) || edge(raw.mode, prev.mode),
            connected,
        };
        self.previous = raw;
        frame
    }
}

pub struct Gamepads {
    gilrs: Option<Gilrs>,
    translator: Translator,
    messages: Vec<String>,
}

impl Gamepads {
    pub fn new() -> Self {
        let gilrs = match Gilrs::new() {
            Ok(gilrs) => Some(gilrs),
            Err(err) => {
                eprintln!("Manettes indisponibles : {err}");
                None
            }
        };
        Gamepads {
            gilrs,
            translator: Translator::default(),
            messages: Vec::new(),
        }
    }

    /// Lit toutes les manettes. À appeler une fois par image.
    pub fn poll(&mut self, now: f64) -> PadFrame {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return PadFrame::default();
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
                EventType::Connected => {
                    let name = gilrs.gamepad(event.id).name().to_string();
                    self.messages.push(format!("Manette connectée : {name}"));
                }
                EventType::Disconnected => self.messages.push("Manette déconnectée".into()),
                _ => {}
            }
        }

        let mut raw = Raw::default();
        let mut connected = false;
        for (_, pad) in gilrs.gamepads() {
            connected = true;
            let x = pad.value(Axis::LeftStickX);
            let y = pad.value(Axis::LeftStickY);
            raw.up |= pad.is_pressed(Pad::DPadUp) || y > STICK_THRESHOLD;
            raw.down |= pad.is_pressed(Pad::DPadDown) || y < -STICK_THRESHOLD;
            raw.left |= pad.is_pressed(Pad::DPadLeft) || x < -STICK_THRESHOLD;
            raw.right |= pad.is_pressed(Pad::DPadRight) || x > STICK_THRESHOLD;
            raw.south |= pad.is_pressed(Pad::South);
            raw.east |= pad.is_pressed(Pad::East);
            raw.start |= pad.is_pressed(Pad::Start);
            raw.select |= pad.is_pressed(Pad::Select);
            raw.mode |= pad.is_pressed(Pad::Mode);
            raw.lb |= pad.is_pressed(Pad::LeftTrigger);
            raw.rb |= pad.is_pressed(Pad::RightTrigger);
        }
        self.translator.frame(raw, now, connected)
    }

    /// Messages à afficher (branchement, débranchement).
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn press_fires_once_then_repeats_after_delay() {
        let mut t = Translator::default();
        let down = Raw {
            down: true,
            ..Raw::default()
        };
        assert!(t.frame(down, 0.0, true).nav.down);
        assert!(!t.frame(down, 0.1, true).nav.down);
        assert!(!t.frame(down, 0.3, true).nav.down);
        assert!(t.frame(down, 0.36, true).nav.down, "première répétition");
        assert!(!t.frame(down, 0.40, true).nav.down);
        assert!(
            t.frame(down, 0.45, true).nav.down,
            "répétitions rapides ensuite"
        );
        assert!(!t.frame(Raw::default(), 0.5, true).nav.down);
    }

    #[test]
    fn buttons_map_to_console() {
        let mut t = Translator::default();
        let raw = Raw {
            south: true,
            east: true,
            left: true,
            ..Raw::default()
        };
        let frame = t.frame(raw, 0.0, true);
        assert!(frame.held.contains(&Button::A));
        assert!(frame.held.contains(&Button::B));
        assert!(frame.held.contains(&Button::Left));
        assert!(frame.nav.confirm && frame.nav.back);
        // Maintenus : pas de nouvelle validation.
        assert!(!t.frame(raw, 0.1, true).nav.confirm);
    }

    #[test]
    fn select_plus_start_exits_without_confirming() {
        let mut t = Translator::default();
        let select = Raw {
            select: true,
            ..Raw::default()
        };
        assert!(!t.frame(select, 0.0, true).exit_game);
        let both = Raw {
            select: true,
            start: true,
            ..Raw::default()
        };
        let frame = t.frame(both, 0.1, true);
        assert!(frame.exit_game);
        assert!(!frame.nav.confirm);
        assert!(!t.frame(both, 0.2, true).exit_game, "une seule fois");
    }
}

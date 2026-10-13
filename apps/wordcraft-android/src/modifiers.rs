//! Modifier keys, tracked by the shell because the platform never reports them.
//!
//! # The gap
//!
//! winit 0.30.13's Android backend emits exactly six window events — `Focused`, `Resized`,
//! `ScaleFactorChanged`, `RedrawRequested`, `Touch` and `KeyboardInput`
//! (`src/platform_impl/android/mod.rs`) — and **never** `WindowEvent::ModifiersChanged`. egui-winit
//! updates its modifier state from that one event and nowhere else, and egui 0.36 carries
//! modifiers as an *event* (`RawInput` has no `modifiers` field any more), so on Android
//! `ctx.input(|i| i.modifiers)` is the all-false default for the whole run:
//!
//! - Shift+click selects a single cell instead of extending the selection,
//! - Ctrl+C / Ctrl+V / Ctrl+S and every other modifier shortcut does nothing,
//! - Alt keytips never come up.
//!
//! The *key events themselves* do arrive, so the modifiers can be tracked from them — which is what
//! a desktop backend does internally. This module does that and reports the state the way egui
//! expects: `Event::ModifiersChanged` (egui's `InputState` reads modifiers from that event alone)
//! plus the per-event `modifiers` copies that `Event::Key` and `Event::PointerButton` carry.
//!
//! Call [`Tracked::apply`] from the shell's `eframe::App::raw_input_hook`, *before* forwarding to
//! the app's own hook, so the app sees the same frames it would on the desktop.
//!
//! # What it does not do
//!
//! The Super/Windows key is deliberately not tracked: on Windows and Linux egui's `command`
//! modifier is Ctrl and `mac_cmd` is documented as macOS-only, so there is no modifier here for it
//! to become. Alt is tracked and reported — it is what opens keytips — but egui's own note that
//! Alt+Shift is how macOS types special characters does not apply on Android.

/// The three modifiers that mean something on this platform, held between frames.
///
/// Between frames because the key press and the click are usually in *different* frames: the user
/// holds Shift down, and only then taps a cell.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tracked {
    alt: bool,
    ctrl: bool,
    shift: bool,
}

/// A modifier a key event is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Modifier {
    Alt,
    Ctrl,
    Shift,
}

/// Which modifier a key event is *about*, from its logical or its physical key.
///
/// Both have to be consulted, and on Android it is the physical one that matters: winit's
/// logical-key table names `Shift` and `Alt` but **not** `Control` (see
/// `platform_impl/android/keycodes.rs::to_logical`), so a Ctrl press arrives with an unidentified
/// logical key and only its `physical_key` — which maps `CtrlLeft`/`CtrlRight` to
/// `ControlLeft`/`ControlRight` — says what it was. That asymmetry is precisely what a user sees:
/// Shift+click and Shift+arrow work, Ctrl+anything does nothing.
fn modifier(key: &egui::Key, physical_key: &Option<egui::Key>) -> Option<Modifier> {
    for k in std::iter::once(key).chain(physical_key.iter()) {
        match k {
            egui::Key::ShiftLeft | egui::Key::ShiftRight => return Some(Modifier::Shift),
            egui::Key::ControlLeft | egui::Key::ControlRight => return Some(Modifier::Ctrl),
            egui::Key::AltLeft | egui::Key::AltRight => return Some(Modifier::Alt),
            _ => {}
        }
    }
    None
}

impl Tracked {
    /// Fold this frame's events into the tracked state and report it back into them.
    ///
    /// Returns the state as `egui::Modifiers`, for the caller that needs to put it on an event of
    /// its own — the shell's touch-to-wheel bridge does (Ctrl+wheel zooms).
    pub fn apply(&mut self, raw: &mut egui::RawInput) -> egui::Modifiers {
        let events = std::mem::take(&mut raw.events);
        let mut out = Vec::with_capacity(events.len() + 1);
        for mut event in events {
            self.observe(&event);
            // Every event that carries a copy gets the state as it stands at that point in the
            // batch, so a Shift press and a click in the same frame are ordered correctly.
            let modifiers = self.reported();
            match &mut event {
                egui::Event::Key { modifiers: m, .. } | egui::Event::PointerButton { modifiers: m, .. } => *m = modifiers,
                _ => {}
            }
            out.push(event);
        }
        // egui's `InputState` takes modifiers from this event and nowhere else, and a frame with no
        // key events in it — the vast majority — still has to report the state it is holding.
        let reported = self.reported();
        out.push(egui::Event::ModifiersChanged(reported));
        raw.events = out;
        reported
    }

    /// The state as egui wants to see it.
    fn reported(&self) -> egui::Modifiers {
        // On Windows and Linux the shortcut modifier is Ctrl, and `mac_cmd` must stay off there.
        egui::Modifiers { alt: self.alt, ctrl: self.ctrl, shift: self.shift, mac_cmd: false, command: self.ctrl }
    }

    /// Update from one event.
    fn observe(&mut self, event: &egui::Event) {
        match event {
            // A modifier key going down or coming up.
            egui::Event::Key { key, physical_key, pressed, .. } => match modifier(key, physical_key) {
                Some(Modifier::Shift) => self.shift = *pressed,
                Some(Modifier::Ctrl) => self.ctrl = *pressed,
                Some(Modifier::Alt) => self.alt = *pressed,
                None => {}
            },
            // The platform reported them itself. It never does on Android, but egui-winit
            // synthesises one when the window loses focus, and that report is authoritative: it is
            // how a modifier that was held while the app went to the background stops being stuck.
            egui::Event::ModifiersChanged(reported) => {
                self.shift = reported.shift;
                self.ctrl = reported.ctrl;
                self.alt = reported.alt;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers};

    fn key(k: Key, physical: Option<Key>, pressed: bool) -> Event {
        Event::Key { key: k, physical_key: physical, pressed, repeat: false, modifiers: Modifiers::default() }
    }

    fn run(t: &mut Tracked, events: Vec<Event>) -> egui::RawInput {
        let mut raw = egui::RawInput { events, ..Default::default() };
        t.apply(&mut raw);
        raw
    }

    /// What the frame reports: egui reads modifiers from `ModifiersChanged` and nowhere else, so every
    /// frame must end with one.
    fn reported(raw: &egui::RawInput) -> Modifiers {
        match raw.events.last() {
            Some(Event::ModifiersChanged(m)) => *m,
            other => panic!("a frame must end with ModifiersChanged: {other:?}"),
        }
    }

    /// The bug this module exists for, in the form Android reports it: winit's logical-key table does
    /// not name the Control keys, so when the physical key is the only one that identifies them the
    /// event must still be understood. Tracking only the logical key is what left every Ctrl shortcut
    /// dead while Shift worked.
    #[test]
    fn ctrl_is_understood_from_either_the_logical_or_the_physical_key() {
        assert_eq!(modifier(&Key::ControlLeft, &None), Some(Modifier::Ctrl));
        assert_eq!(modifier(&Key::ControlRight, &None), Some(Modifier::Ctrl));
        // The case that matters: the *logical* key says nothing useful (here: an ordinary A) and the
        // physical key is what identifies the modifier.
        assert_eq!(modifier(&Key::A, &Some(Key::ControlLeft)), Some(Modifier::Ctrl));
        assert_eq!(modifier(&Key::ShiftLeft, &None), Some(Modifier::Shift));
        assert_eq!(modifier(&Key::A, &Some(Key::A)), None, "an ordinary key is not a modifier");
    }

    /// Tracked and released through `apply`, and the state persists across frames with no events
    /// (which is most frames — the key press and the shortcut it enables are usually in different ones).
    #[test]
    fn a_held_modifier_is_reported_in_frames_with_no_events() {
        let mut t = Tracked::default();
        let m = reported(&run(&mut t, vec![key(Key::ControlLeft, Some(Key::ControlLeft), true)]));
        assert!(m.ctrl && m.command, "Ctrl is egui's command modifier: {m:?}");
        assert!(!m.mac_cmd && !m.shift && !m.alt);
        let quiet = run(&mut t, vec![]);
        assert!(reported(&quiet).ctrl, "a frame with no events still reports what is held");
        assert_eq!(quiet.events.len(), 1, "and its only event is that report");
        let released = reported(&run(&mut t, vec![key(Key::ControlLeft, Some(Key::ControlLeft), false)]));
        assert!(!released.ctrl, "released");
    }

    /// Order inside one batch: a press applies to the events after it, not before it.
    #[test]
    fn the_state_is_the_one_at_that_point_in_the_batch() {
        let click = |pressed: bool| Event::PointerButton {
            pos: egui::pos2(10.0, 10.0),
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::default(),
        };
        let mut t = Tracked::default();
        let mut raw = egui::RawInput {
            events: vec![click(true), key(Key::ShiftLeft, None, true), click(true)],
            ..Default::default()
        };
        t.apply(&mut raw);
        let carried = |e: &Event| match e {
            Event::PointerButton { modifiers, .. } => *modifiers,
            other => panic!("no modifiers on {other:?}"),
        };
        assert!(!carried(&raw.events[0]).shift, "the first click came before the press");
        assert!(carried(&raw.events[2]).shift, "the second came after it");
    }
}

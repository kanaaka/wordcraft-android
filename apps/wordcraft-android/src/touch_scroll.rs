//! Two-finger scrolling for the shell: the platform sends touches, the apps want a wheel.
//!
//! # The gap
//!
//! Every app in this suite scrolls from `egui::Event::MouseWheel`. WordCraft's `scroll.rs` is
//! explicit about what it expects — pixel deltas move the page 1:1, `TouchPhase::End` on the last
//! event is the "fingers lifted" signal that starts the kinetic coast — and it is written for a
//! *touch* device: it handles phases, synthesises momentum where the platform does not, and takes
//! wheel notches as three lines. None of that ever runs on Android, because winit's Android backend
//! has no mouse and therefore emits no wheel events at all (`src/platform_impl/android/mod.rs` sends
//! six window events and a wheel is not one of them). Touch arrives as `WindowEvent::Touch`, which
//! egui-winit forwards as `egui::Event::Touch` *and* turns into an emulated mouse pointer — so a
//! finger drags a selection, and the page never moves. Exactly the complaint a tablet user makes.
//!
//! # What this does
//!
//! Turns a **two-finger drag** into wheel events and leaves one finger alone:
//!
//! - one finger: untouched, so dragging still selects text — the emulated pointer works as before;
//! - two or more fingers: the centroid's movement becomes `Event::MouseWheel` with
//!   `MouseWheelUnit::Point` and `TouchPhase::Move`, and a final `TouchPhase::End` event when the
//!   fingers lift, which is what makes the apps coast to a stop;
//! - while a two-finger gesture is live the emulated **pointer button** events are dropped, or the
//!   first finger would drag a selection across the page at the same time as it scrolled. The
//!   pointer *position* events are kept: the apps only scroll while the pointer is over the canvas.
//!
//! The touch events themselves are left in the batch — egui's pinch handling (`zoom_delta`) reads
//! them and the apps zoom with it, so a pinch keeps working and a two-finger *pan* moves nothing but
//! the page.
//!
//! Two fingers is the convention a tablet user already knows from every document viewer, and it
//! needs nothing from inside the apps: as far as their scroll code is concerned, a touchpad moved.
//!
//! Call [`TouchScroll::apply`] from the shell's `eframe::App::raw_input_hook`, after the modifier
//! tracking, whose reported state the wheel event has to carry (Ctrl+wheel zooms).

use std::collections::HashMap;

/// Touch state between frames: where each finger is (a scroll is a *change* of position and a batch
/// only carries the new one), and how far the last scroll was measured from.
#[derive(Debug, Default)]
pub struct TouchScroll {
    /// Keyed by the raw id: `egui::TouchId` is a `u64` newtype with no `Hash`,
    /// so it cannot be a map key itself.
    fingers: HashMap<u64, egui::Pos2>,
    /// The centroid the last scroll was measured from. `None` outside a gesture, so the moment a
    /// second finger lands — which moves the centroid by half the gap between the fingers — is not
    /// itself a scroll.
    last: Option<egui::Pos2>,
    /// A two-finger gesture has scrolled and has not ended: the apps are coasting on the last
    /// `TouchPhase::End` and must be told when this one finishes.
    scrolling: bool,
}

impl TouchScroll {
    /// Rewrite one frame's events so the apps can scroll from them.
    ///
    /// `modifiers` is the shell's tracked state, because egui reads the zoom modifier from the event
    /// itself (and the platform never reports modifiers — see `modifiers.rs`).
    pub fn apply(&mut self, raw: &mut egui::RawInput, modifiers: egui::Modifiers) {
        let events = std::mem::take(&mut raw.events);

        // First the finger bookkeeping, over the whole batch: the events of one frame arrive in
        // order, and the delta wanted is between this frame's centroid and the last one.
        for event in &events {
            if let egui::Event::Touch { id, phase, pos, .. } = event {
                match phase {
                    egui::TouchPhase::Start | egui::TouchPhase::Move => {
                        self.fingers.insert(id.0, *pos);
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.fingers.remove(&id.0);
                    }
                }
            }
        }

        let centroid = self.centroid();
        let gesture = self.fingers.len() >= 2;
        // Positive is "the content moves down/right", which is what egui documents and what a finger
        // dragged down the screen means.
        let delta = match (gesture, centroid, self.last) {
            (true, Some(now), Some(prev)) => now - prev,
            _ => egui::Vec2::ZERO,
        };

        let mut out = Vec::with_capacity(events.len() + 1);
        for event in events {
            if gesture && matches!(event, egui::Event::PointerButton { .. }) {
                continue;
            }
            out.push(event);
        }

        if gesture && delta != egui::Vec2::ZERO {
            self.scrolling = true;
            out.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta,
                phase: egui::TouchPhase::Move,
                modifiers,
            });
        } else if self.scrolling && !gesture {
            // The fingers lifted. The apps' momentum code coasts from this, and a flick that was
            // fast enough keeps the page moving after the hand is off it.
            self.scrolling = false;
            out.push(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::Vec2::ZERO,
                phase: egui::TouchPhase::End,
                modifiers,
            });
        }

        self.last = if gesture { centroid } else { None };
        raw.events = out;
    }

    /// Where the fingers are, on average. `None` with none down.
    fn centroid(&self) -> Option<egui::Pos2> {
        let n = self.fingers.len();
        if n == 0 {
            return None;
        }
        let sum = self.fingers.values().fold(egui::Vec2::ZERO, |a, p| a + p.to_vec2());
        Some((sum / n as f32).to_pos2())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(id: u64, phase: egui::TouchPhase, x: f32, y: f32) -> egui::Event {
        egui::Event::Touch {
            device_id: egui::TouchDeviceId(0),
            id: egui::TouchId::from(id),
            phase,
            pos: egui::pos2(x, y),
            force: None,
        }
    }

    fn moved(x: f32, y: f32) -> egui::Event {
        egui::Event::PointerMoved(egui::pos2(x, y))
    }

    fn press(x: f32, y: f32) -> egui::Event {
        egui::Event::PointerButton { pos: egui::pos2(x, y), button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() }
    }

    fn wheels(raw: &egui::RawInput) -> Vec<(egui::Vec2, egui::TouchPhase)> {
        raw.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { delta, phase, .. } => Some((*delta, *phase)),
                _ => None,
            })
            .collect()
    }

    fn batch(events: Vec<egui::Event>) -> egui::RawInput {
        egui::RawInput { events, ..Default::default() }
    }

    #[test]
    fn one_finger_still_selects_and_never_scrolls() {
        let mut s = TouchScroll::default();
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), moved(100.0, 100.0), press(100.0, 100.0)]);
        s.apply(&mut raw, Default::default());
        assert!(
            raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: true, .. })),
            "one finger keeps its press: that is how a tablet drags a selection"
        );
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 140.0, 220.0), moved(140.0, 220.0)]);
        s.apply(&mut raw, Default::default());
        assert!(wheels(&raw).is_empty(), "one finger does not scroll");
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))), "and the drag reaches the app as a pointer drag");
    }

    #[test]
    fn two_fingers_scroll_and_the_press_is_dropped() {
        let mut s = TouchScroll::default();
        // Finger one lands (the emulated press is what selects on a tablet).
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0), press(100.0, 100.0)]);
        s.apply(&mut raw, Default::default());
        // Finger two lands: no scroll yet — the centroid jumped, but nothing has moved.
        let mut raw = batch(vec![touch(2, egui::TouchPhase::Start, 200.0, 100.0)]);
        s.apply(&mut raw, Default::default());
        assert!(wheels(&raw).is_empty(), "a second finger landing is not a scroll");
        // Both drag down 30 points: the page moves down 30, and no press comes with it.
        let mut raw = batch(vec![
            touch(1, egui::TouchPhase::Move, 100.0, 130.0),
            touch(2, egui::TouchPhase::Move, 200.0, 130.0),
            moved(100.0, 130.0),
            press(100.0, 130.0),
        ]);
        s.apply(&mut raw, Default::default());
        let w = wheels(&raw);
        assert_eq!(w.len(), 1, "one wheel event per frame");
        assert!((w[0].0.y - 30.0).abs() < 1e-3, "the content follows the fingers: {:?}", w[0].0);
        assert_eq!(w[0].1, egui::TouchPhase::Move);
        assert!(!raw.events.iter().any(|e| matches!(e, egui::Event::PointerButton { .. })), "no selection drag while scrolling");
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::PointerMoved(_))), "the pointer position stays: the apps only scroll while the pointer is over the canvas");
        // Both lift: the release the apps coast from.
        let mut raw = batch(vec![
            touch(1, egui::TouchPhase::End, 100.0, 130.0),
            touch(2, egui::TouchPhase::End, 200.0, 130.0),
        ]);
        s.apply(&mut raw, Default::default());
        let w = wheels(&raw);
        assert_eq!(w.len(), 1, "the fingers lifting is reported");
        assert_eq!(w[0].1, egui::TouchPhase::End);
        assert_eq!(w[0].0, egui::Vec2::ZERO, "a lift carries no distance of its own");
        // And the next frame is quiet: one release, not one per frame.
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Start, 100.0, 100.0)]);
        s.apply(&mut raw, Default::default());
        assert!(wheels(&raw).is_empty());
    }

    #[test]
    fn a_pinch_does_not_scroll_the_page() {
        let mut s = TouchScroll::default();
        s.apply(&mut batch(vec![touch(1, egui::TouchPhase::Start, 400.0, 400.0), touch(2, egui::TouchPhase::Start, 500.0, 400.0)]), Default::default());
        // Fingers move apart around a fixed centre: that is egui's zoom_delta, not a scroll.
        let mut raw = batch(vec![touch(1, egui::TouchPhase::Move, 300.0, 400.0), touch(2, egui::TouchPhase::Move, 600.0, 400.0)]);
        s.apply(&mut raw, Default::default());
        assert!(wheels(&raw).is_empty(), "a pinch zooms, it does not scroll: {:?}", wheels(&raw));
        assert!(raw.events.iter().any(|e| matches!(e, egui::Event::Touch { .. })), "the touch events stay in the batch, egui reads them for zoom");
    }
}

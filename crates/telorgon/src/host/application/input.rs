use crate::foundation::{PointF, SizeF};
use crate::input::{InputEvent, PointerDeviceKind, PointerId, ScrollPrecision};

pub const LISTEN_POINTER: u16 = 1 << 0;
pub const LISTEN_ACTION: u16 = 1 << 1;
pub const LISTEN_KEY: u16 = 1 << 2;
pub const LISTEN_FOCUS: u16 = 1 << 3;

#[derive(Clone, Debug, PartialEq)]
pub enum PlatformInput {
    Input(InputEvent),
    Resize(SizeF),
}

impl From<InputEvent> for PlatformInput {
    fn from(event: InputEvent) -> Self {
        Self::Input(event)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputCoalescer {
    pointer: Option<InputEvent>,
    scroll: Option<(PointerId, PointerDeviceKind, PointF, ScrollPrecision)>,
    resize: Option<SizeF>,
    ordered: Vec<PlatformInput>,
    diagnostics: InputCoalescingDiagnostics,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct InputCoalescingDiagnostics {
    pub(crate) events_received: u64,
    pub(crate) pointer_moves_received: u64,
    pub(crate) pointer_moves_coalesced: u64,
    pub(crate) scroll_events_received: u64,
    pub(crate) scroll_events_coalesced: u64,
    pub(crate) resize_events_received: u64,
    pub(crate) resize_events_coalesced: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct InputBatch {
    pub(crate) events: Vec<PlatformInput>,
    pub(crate) diagnostics: InputCoalescingDiagnostics,
}

impl InputCoalescer {
    pub(crate) fn push(&mut self, event: PlatformInput) {
        self.diagnostics.events_received = self.diagnostics.events_received.saturating_add(1);
        match event {
            PlatformInput::Input(
                event @ InputEvent::PointerMoved {
                    pointer, device, ..
                },
            ) => {
                self.flush_scroll();
                self.diagnostics.pointer_moves_received =
                    self.diagnostics.pointer_moves_received.saturating_add(1);
                if !matches!(
                    self.pointer.as_ref(),
                    Some(InputEvent::PointerMoved {
                        pointer: pending_pointer,
                        device: pending_device,
                        ..
                    }) if *pending_pointer == pointer && *pending_device == device
                ) {
                    self.flush_pointer();
                } else {
                    self.diagnostics.pointer_moves_coalesced =
                        self.diagnostics.pointer_moves_coalesced.saturating_add(1);
                }
                self.pointer = Some(event);
            }
            PlatformInput::Input(InputEvent::Scroll {
                pointer,
                device,
                delta,
                precision,
            }) => {
                self.flush_pointer();
                self.diagnostics.scroll_events_received =
                    self.diagnostics.scroll_events_received.saturating_add(1);
                if let Some((pending_pointer, pending_device, pending_delta, pending_precision)) = &mut self.scroll
                    && *pending_pointer == pointer
                    && *pending_device == device
                    && *pending_precision == precision
                    // A reversal must cancel old motion, not disappear in a summed delta.
                    && pending_delta.x * delta.x >= 0.0
                    && pending_delta.y * delta.y >= 0.0
                {
                    pending_delta.x += delta.x;
                    pending_delta.y += delta.y;
                    self.diagnostics.scroll_events_coalesced =
                        self.diagnostics.scroll_events_coalesced.saturating_add(1);
                } else {
                    self.flush_scroll();
                    self.scroll = Some((pointer, device, delta, precision));
                }
            }
            PlatformInput::Resize(size) => {
                self.diagnostics.resize_events_received =
                    self.diagnostics.resize_events_received.saturating_add(1);
                if self.resize.replace(size).is_some() {
                    self.diagnostics.resize_events_coalesced =
                        self.diagnostics.resize_events_coalesced.saturating_add(1);
                }
            }
            ordered => {
                self.flush_transient();
                self.ordered.push(ordered);
            }
        }
    }

    pub(crate) fn has_pending(&self) -> bool {
        self.pointer.is_some()
            || self.scroll.is_some()
            || self.resize.is_some()
            || !self.ordered.is_empty()
    }

    #[cfg(any(feature = "profiler", test))]
    pub(crate) fn has_only_pending_pointer_moves(&self) -> bool {
        self.diagnostics.events_received != 0
            && self.diagnostics.events_received == self.diagnostics.pointer_moves_received
    }

    pub(crate) fn drain(&mut self) -> InputBatch {
        let mut events = std::mem::take(&mut self.ordered);
        if let Some(size) = self.resize.take() {
            events.insert(0, PlatformInput::Resize(size));
        }
        self.flush_transient();
        events.append(&mut self.ordered);
        InputBatch {
            events,
            diagnostics: std::mem::take(&mut self.diagnostics),
        }
    }

    pub(crate) fn recycle(&mut self, mut events: Vec<PlatformInput>) {
        events.clear();
        self.ordered = events;
    }

    fn flush_transient(&mut self) {
        self.flush_pointer();
        self.flush_scroll();
    }

    fn flush_pointer(&mut self) {
        if let Some(event) = self.pointer.take() {
            self.ordered.push(PlatformInput::Input(event));
        }
    }

    fn flush_scroll(&mut self) {
        if let Some((pointer, device, delta, precision)) = self.scroll.take() {
            self.ordered.push(PlatformInput::Input(InputEvent::Scroll {
                pointer,
                device,
                delta,
                precision,
            }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_bursts_keep_only_the_latest_extent() {
        let mut input = InputCoalescer::default();
        input.push(PlatformInput::Resize(SizeF {
            width: 640.0,
            height: 480.0,
        }));
        input.push(PlatformInput::Resize(SizeF {
            width: 900.0,
            height: 700.0,
        }));
        input.push(PlatformInput::Resize(SizeF {
            width: 1280.0,
            height: 800.0,
        }));

        let batch = input.drain();
        assert_eq!(
            batch.events,
            vec![PlatformInput::Resize(SizeF {
                width: 1280.0,
                height: 800.0,
            })]
        );
        assert_eq!(batch.diagnostics.events_received, 3);
        assert_eq!(batch.diagnostics.resize_events_received, 3);
        assert_eq!(batch.diagnostics.resize_events_coalesced, 2);
        assert!(!input.has_pending());
    }

    #[test]
    fn consecutive_pointer_moves_keep_the_latest_position_and_report_collapses() {
        let mut input = InputCoalescer::default();
        for value in 0..10_000 {
            input.push(
                InputEvent::mouse_moved(PointF {
                    x: value as f32,
                    y: (value + 1) as f32,
                })
                .into(),
            );
        }

        assert!(input.has_pending());
        assert!(input.has_only_pending_pointer_moves());
        let batch = input.drain();
        assert_eq!(batch.events.len(), 1);
        assert_eq!(batch.diagnostics.pointer_moves_received, 10_000);
        assert_eq!(batch.diagnostics.pointer_moves_coalesced, 9_999);
        assert_eq!(
            batch.events[0],
            PlatformInput::Input(InputEvent::mouse_moved(PointF {
                x: 9_999.0,
                y: 10_000.0,
            }))
        );
    }

    #[test]
    fn ordered_button_events_fence_pointer_move_coalescing() {
        use crate::input::{ButtonState, PointerButton};

        let mut input = InputCoalescer::default();
        input.push(InputEvent::mouse_moved(PointF { x: 1.0, y: 1.0 }).into());
        input.push(InputEvent::mouse_button(PointerButton::PRIMARY, ButtonState::Pressed).into());
        input.push(InputEvent::mouse_moved(PointF { x: 2.0, y: 2.0 }).into());

        assert!(!input.has_only_pending_pointer_moves());

        let batch = input.drain();
        assert_eq!(batch.events.len(), 3);
        assert!(matches!(
            batch.events[0],
            PlatformInput::Input(InputEvent::PointerMoved { position, .. })
                if position == (PointF { x: 1.0, y: 1.0 })
        ));
        assert!(matches!(
            batch.events[1],
            PlatformInput::Input(InputEvent::PointerButton { .. })
        ));
        assert!(matches!(
            batch.events[2],
            PlatformInput::Input(InputEvent::PointerMoved { position, .. })
                if position == (PointF { x: 2.0, y: 2.0 })
        ));
        assert_eq!(batch.diagnostics.pointer_moves_coalesced, 0);
    }
    #[test]
    fn wheel_reversals_precision_and_pointer_targets_fence_coalescing() {
        let events = vec![
            InputEvent::mouse_wheel(PointF { x: 0.0, y: -1.0 }),
            InputEvent::mouse_wheel(PointF { x: 0.0, y: 1.0 }),
            InputEvent::mouse_scroll(PointF { x: 0.0, y: 0.25 }),
            InputEvent::mouse_moved(PointF { x: 50.0, y: 50.0 }),
            InputEvent::mouse_scroll(PointF { x: 0.0, y: 0.25 }),
        ];
        let mut input = InputCoalescer::default();
        for event in &events {
            input.push(event.clone().into());
        }
        assert_eq!(
            input.drain().events,
            events
                .into_iter()
                .map(PlatformInput::Input)
                .collect::<Vec<_>>()
        );
    }
}

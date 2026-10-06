#![cfg(any(feature = "application-software", feature = "application-vulkan"))]

use std::num::NonZeroU16;

use telorgon::platform_winit::{
    MAX_WINIT_VIEWS, ViewRegistry, ViewRegistryError, ViewRegistryLimitError,
};
use winit::window::WindowId;

fn maximum(value: u16) -> NonZeroU16 {
    NonZeroU16::new(value).unwrap()
}

fn window(value: u64) -> WindowId {
    WindowId::from(value)
}

#[test]
fn registry_is_bounded_and_maps_multiple_windows_in_both_directions() {
    assert_eq!(
        ViewRegistry::new(maximum(MAX_WINIT_VIEWS + 1)).unwrap_err(),
        ViewRegistryLimitError {
            requested: MAX_WINIT_VIEWS + 1,
            maximum: MAX_WINIT_VIEWS,
        }
    );

    let mut registry = ViewRegistry::new(maximum(2)).unwrap();
    let first = registry.register(window(11)).unwrap();
    let second = registry.register(window(22)).unwrap();

    assert_eq!(registry.view_for_window(window(11)), Some(first.view));
    assert_eq!(registry.window_for_view(second.view), Some(window(22)));
    assert_eq!(
        registry.iter().collect::<Vec<_>>(),
        vec![(first.view, window(11)), (second.view, window(22))]
    );
    assert_eq!(registry.len(), 2);
    assert_eq!(
        registry.register(window(33)),
        Err(ViewRegistryError::CapacityReached {
            maximum: maximum(2),
        })
    );
    assert_eq!(registry.len(), 2);
}

#[test]
fn duplicate_registration_and_conflicting_replacement_are_atomic() {
    let mut registry = ViewRegistry::new(maximum(2)).unwrap();
    let first = registry.register(window(11)).unwrap();
    let second = registry.register(window(22)).unwrap();

    assert_eq!(
        registry.register(window(11)),
        Err(ViewRegistryError::WindowAlreadyRegistered {
            window: window(11),
            view: first.view,
        })
    );
    assert_eq!(
        registry.replace_window(first.view, window(11), window(22)),
        Err(ViewRegistryError::WindowAlreadyRegistered {
            window: window(22),
            view: second.view,
        })
    );
    assert_eq!(registry.window_for_view(first.view), Some(window(11)));
    assert_eq!(registry.view_for_window(window(22)), Some(second.view));
}

#[test]
fn window_replacement_preserves_the_logical_view_and_rejects_stale_identity() {
    let mut registry = ViewRegistry::new(maximum(1)).unwrap();
    let registration = registry.register(window(11)).unwrap();

    let replacement = registry
        .replace_window(registration.view, window(11), window(12))
        .unwrap();
    assert_eq!(replacement.view, registration.view);
    assert_eq!(replacement.previous_window, window(11));
    assert_eq!(replacement.current_window, window(12));
    assert_eq!(registry.view_for_window(window(11)), None);
    assert_eq!(
        registry.view_for_window(window(12)),
        Some(registration.view)
    );

    assert_eq!(
        registry.retire(registration.view, window(11)),
        Err(ViewRegistryError::WindowMismatch {
            view: registration.view,
            expected_window: window(11),
            registered_window: window(12),
        })
    );
    assert_eq!(
        registry.window_for_view(registration.view),
        Some(window(12))
    );
}

#[test]
fn retirement_and_slot_reuse_advance_generation_and_isolate_stale_callbacks() {
    let mut registry = ViewRegistry::new(maximum(1)).unwrap();
    let retired_generation = registry.register(window(11)).unwrap();
    registry
        .retire(retired_generation.view, retired_generation.window)
        .unwrap();

    assert_eq!(registry.window_for_view(retired_generation.view), None);
    assert_eq!(registry.view_for_window(retired_generation.window), None);

    let current_generation = registry.register(window(22)).unwrap();
    assert_eq!(
        current_generation.view.slot(),
        retired_generation.view.slot()
    );
    assert_eq!(
        current_generation.view.generation(),
        retired_generation.view.generation() + 1
    );
    assert_ne!(current_generation.view, retired_generation.view);
    assert_eq!(
        registry.retire(retired_generation.view, window(11)),
        Err(ViewRegistryError::ViewUnavailable {
            view: retired_generation.view,
        })
    );
    assert_eq!(
        registry.window_for_view(current_generation.view),
        Some(window(22))
    );
}

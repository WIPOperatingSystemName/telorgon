use super::*;
use crate::graphics::render::RenderScene;

fn item(value: u8) -> TrayItem {
    TrayItem {
        id: TrayItemId("test-icon".into()),
        title: String::new(),
        tooltip: String::new(),
        icon: TrayImage::pixels(TrayPixels::new(1, 1, vec![value, 0, 0, 255]).unwrap()),
        image_id: 0,
        image_revision: 0,
        status: TrayStatus::Active,
        item_is_menu: false,
        has_menu: false,
    }
}

#[test]
fn changing_and_restoring_tray_pixels_never_decreases_resource_version() {
    let mut scene = RenderScene::default();
    let mut previous = None;
    let mut identity = None;
    // A -> B -> A necessarily reverses a hash-based ordering on one update.
    for (value, expected_version) in [(40, 1), (200, 2), (40, 3), (40, 3)] {
        let mut next = item(value);
        retain_icon_identity(&mut next, previous.as_ref());
        let resource = next.image_resource().unwrap();
        assert_eq!(resource.content_version, expected_version);
        assert_eq!(*identity.get_or_insert(resource.image), resource.image);
        scene.set_image_resource(resource).unwrap();
        previous = Some(next);
    }
    let mut named = previous.as_ref().unwrap().clone();
    named.icon = TrayImage::named("named-icon");
    retain_icon_identity(&mut named, previous.as_ref());
    assert!(named.image_resource().is_none());
    let mut restored = item(40);
    retain_icon_identity(&mut restored, Some(&named));
    let resource = restored.image_resource().unwrap();
    assert_eq!(resource.content_version, 5);
    scene.set_image_resource(resource).unwrap();
}

#[test]
fn text_only_refresh_keeps_pixels_stable_and_new_registration_gets_new_identity() {
    let mut first = item(40);
    retain_icon_identity(&mut first, None);
    let mut changed = first.clone();
    changed.title = "new title".into();
    changed.tooltip = "new tooltip".into();
    retain_icon_identity(&mut changed, Some(&first));
    assert_eq!(first.image_resource(), changed.image_resource());
    let mut replacement = item(40);
    retain_icon_identity(&mut replacement, None);
    assert_ne!(first.image_id, replacement.image_id);
    assert_eq!(replacement.image_revision, 1);
}

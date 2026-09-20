use telorgon::app::*;
use telorgon::{Easing, tween_ms};

#[test]
fn shell_widget_api_accepts_glass_border_and_independent_preview_motion() {
    let widget = WindowTiling::snap()
        .halves(true)
        .quadrants(true)
        .shared_resize(true)
        .edge_threshold(16.0)
        .corner_threshold(48.0)
        .divider_hit_width(8.0)
        .preview(TilePreviewDesign {
            fill: Fill::Glass(GlassStyle::liquid()),
            border: telorgon::ui::Border::all(2.0, ColorRgba8::rgba(180, 210, 255, 220)),
            corner_radius: 14.0,
            padding: Insets::all(8.0),
            motion: TilePreviewMotion::smooth()
                .appear(tween_ms(150, Easing::EaseOut))
                .relocate(tween_ms(200, Easing::EaseOut))
                .disappear(tween_ms(90, Easing::EaseOut)),
        });
    fn accepts_widget<W: ShellWidget>(_: &W) {}
    accepts_widget(&widget);
    let spec = widget.surface();
    assert_eq!(spec.pointer, ShellPointer::PassThrough);
    assert_eq!(spec.focus, ShellFocus::None);
    assert!(!spec.visible);
    spec.validate().unwrap();
    let _ = widget.view().into_element();
    let _ = ShellWindowAction::Snap(TileTarget::BottomRight);
    let _ = ShellWindowAction::Float;
}

#[test]
fn preview_padding_defaults_to_zero_and_rejects_invalid_sides() {
    assert_eq!(TilePreviewDesign::default().padding, Insets::ZERO);
    for side in 0..4 {
        for invalid in [-1.0, f32::NAN, f32::INFINITY] {
            let mut values = [0.0; 4];
            values[side] = invalid;
            assert!(
                std::panic::catch_unwind(|| WindowTiling::snap().preview(TilePreviewDesign {
                    padding: Insets::new(values[0], values[1], values[2], values[3]),
                    ..Default::default()
                }))
                .is_err()
            );
        }
    }
}

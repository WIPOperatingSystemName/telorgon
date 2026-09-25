use telorgon::app::*;

#[test]
fn previews_share_fill_but_keep_independent_borders() {
    const FILL: Fill = Fill::Glass(GlassStyle::liquid());
    const RESIZE: ResizePreviewDesign = ResizePreviewDesign::new(FILL);
    let tile = TilePreviewDesign {
        fill: FILL,
        border: Border::all(2.0, ColorRgba8::rgba(255, 255, 255, 128)),
        ..Default::default()
    };
    assert_eq!(RESIZE.fill, tile.fill);
    assert_eq!(RESIZE.border, Border::default());
    assert_ne!(RESIZE.border, tile.border);
    assert_eq!(Fill::None.color().a, 0);
    let content = telorgon::WindowContentStyle {
        background: ColorRgba8::rgba(0, 0, 0, 0),
        corner_radius: 0.0,
        resize_preview: Some(ResizePreviewDesign {
            fill: Fill::None,
            border: tile.border,
            corner_radius: 12.0,
        }),
    };
    assert_eq!(content.resize_preview.unwrap().border, tile.border);
}

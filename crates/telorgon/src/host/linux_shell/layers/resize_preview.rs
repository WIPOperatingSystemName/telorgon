use super::*;

pub(in crate::host::linux_shell) fn resize_preview_radii(
    window: &ClientWindow,
    appearance: crate::ResizePreviewDesign,
) -> crate::ui::CornerRadii {
    crate::ui::CornerRadii::all(
        if window.maximized || window.tile.is_some() || window.fullscreen {
            0.0
        } else {
            appearance.corner_radius
        },
    )
}

pub(super) fn layers(
    surface: WaylandSurfaceId,
    window: &ClientWindow,
    appearance: crate::ResizePreviewDesign,
    frame_border: Option<&BoxInstance>,
    position: PointI,
    outer: SizeI,
) -> Vec<ShellLayer> {
    let mut preview = ShellLayer::solid(
        ShellLayerKey::ResizeVeil(surface.get()),
        ShellSceneKey::ResizeVeil(surface.get()),
        appearance.fill.color(),
        RectI {
            x: position.x,
            y: position.y,
            width: outer.width,
            height: outer.height,
        },
    );
    if let crate::Fill::Glass(style) = appearance.fill {
        preview.glass = Some(style.normalized());
    }
    if let Some(border) = frame_border {
        preview = preview.with_frame_outline(border, position);
    } else {
        preview.rounded_clips = [
            Some(crate::graphics::render::RoundedClip::new(
                crate::foundation::RectF {
                    x: preview.target.x as f32,
                    y: preview.target.y as f32,
                    width: preview.target.width as f32,
                    height: preview.target.height as f32,
                },
                resize_preview_radii(window, appearance),
            )),
            None,
        ];
    }
    let border = ShellLayer::resize_preview_border(surface.get(), &preview, appearance.border);
    std::iter::once(preview).chain(border).collect()
}

#[cfg(all(test, feature = "shell-xwayland", target_env = "gnu"))]
mod tests {
    use super::*;
    use crate::foundation::ColorRgba8;

    #[test]
    fn client_decorated_preview_has_its_own_border_and_contour() {
        use crate::host::linux_shell::client::maximize_preview_tests::test_window;
        use crate::host::linux_shell::scene::ShellLayerContent;
        let size = SizeI {
            width: 640,
            height: 480,
        };
        let position = PointI { x: 100, y: 80 };
        let mut window = test_window(size, position);
        window.server_decorated = false;
        let border = crate::ui::Border::all(2.0, ColorRgba8::rgba(255, 255, 255, 255));
        for fill in [
            crate::Fill::Glass(crate::GlassStyle::liquid()),
            crate::Fill::None,
        ] {
            let design = crate::ResizePreviewDesign {
                fill,
                border,
                corner_radius: 18.0,
            };
            for maximized in [false, true] {
                window.maximized = maximized;
                let preview = layers(
                    WaylandSurfaceId::from_raw(1).unwrap(),
                    &window,
                    design,
                    None,
                    position,
                    size,
                );
                assert_eq!(preview.len(), 2);
                let radii = crate::ui::CornerRadii::all(if maximized { 0.0 } else { 18.0 });
                assert_eq!(preview[0].rounded_clips[0].unwrap().radii, radii);
                let ShellLayerContent::Decoration { instance, .. } = &preview[1].content else {
                    panic!("missing preview border")
                };
                assert_eq!(instance.border, border);
                assert_eq!(instance.corner_radii, radii);
                assert_eq!(preview[0].target, preview[1].target);
                assert!(window.chrome.is_none());
            }
        }
    }
}

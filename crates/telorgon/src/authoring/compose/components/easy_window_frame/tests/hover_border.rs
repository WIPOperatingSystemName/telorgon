use super::*;
use crate::foundation::{Affine2D, MonotonicInstant, PointF, RectI, SizeI};
use crate::graphics::render::{RenderBackend, RoundedClip, frame_border};
use crate::graphics::renderers::software::{
    SoftwareCompositeLayer, SoftwareRenderer, SoftwareSurface,
};
use crate::host::application::AppRuntimeCore;
use crate::shell::window_chrome::WindowChromeSnapshot;

#[test]
fn hovered_flush_controls_leave_border_pixels_unchanged_at_display_scale() {
    for scale in [1.0, 1.25, 2.0] {
        let logical = SizeI {
            width: 200,
            height: 100,
        };
        let extent = SizeI {
            width: (200.0 * scale) as i32,
            height: (100.0 * scale) as i32,
        };
        let mut design = DESIGN;
        design.active.frame_border = Border::all(1.5, ColorRgba8::rgba(65, 68, 75, 255));
        design.active.frame_background = ColorRgba8::rgba(23, 27, 36, 255);
        design.normal.frame_radius = 12.0;
        design.normal.shadow = None;
        design.title_bar.padding = Insets::ZERO;
        design.title_bar.show_client_icon = false;
        design.controls.gap = 0.0;
        for control in [
            &mut design.controls.close,
            &mut design.controls.maximize,
            &mut design.controls.minimize,
        ] {
            control.style.height = Dimension::FILL;
            control.style.resting.decoration = BoxDecoration::new();
            control.style.hovered = Some(WindowControlVisual {
                decoration: BoxDecoration::new()
                    .background(Background::Color(ColorRgba8::rgba(183, 52, 72, 255))),
                ..control.style.resting
            });
            control.style.transition = Some(TransitionSpec {
                duration_ms: 0,
                easing: Easing::Linear,
                repeat: false,
            });
        }
        let mut runtime = AppRuntimeCore::from_composed_with_extent(
            easy_window_frame(design).compose(WindowChromeModel::new(42, "").active(true)),
            logical,
        )
        .unwrap();
        runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
        let snapshot = WindowChromeSnapshot::derive(runtime.ui(), runtime.layout()).unwrap();
        let nodes = runtime
            .ui()
            .style_bindings()
            .iter()
            .filter(|binding| binding.local_style.is_some())
            .map(|binding| binding.state_root)
            .collect::<Vec<_>>();
        let geometry = nodes
            .iter()
            .map(|node| runtime.layout().computed(*node).unwrap().border_rect)
            .collect::<Vec<_>>();
        let transform = Affine2D {
            m11: scale,
            m22: scale,
            ..Affine2D::IDENTITY
        };
        let mut source = SoftwareRenderer.create_scene().unwrap();
        let mut frames = Vec::new();
        let mut contour = None;
        for (step, hovered) in [false, true, false].into_iter().enumerate() {
            for &node in &nodes {
                runtime
                    .ui_mut()
                    .route_interaction_flag(node, InteractionFlags::HOVERED, hovered);
            }
            runtime
                .prepare_frame(
                    MonotonicInstant::from_nanos((step as u64 + 1) * 1_000_000),
                    true,
                )
                .unwrap();
            assert_eq!(
                geometry,
                nodes
                    .iter()
                    .map(|node| runtime.layout().computed(*node).unwrap().border_rect)
                    .collect::<Vec<_>>()
            );
            let mut delta = runtime.scene_snapshot();
            let mut border = delta
                .boxes
                .iter()
                .flat_map(|patch| patch.values.iter())
                .find(|instance| instance.node == snapshot.frame.node)
                .unwrap()
                .clone();
            frame_border::prepare_interior(&mut delta, &border);
            delta.epoch = step as u64 + 1;
            delta.extent.width *= scale;
            delta.extent.height *= scale;
            for patch in &mut delta.spatial_nodes {
                for spatial in std::sync::Arc::make_mut(&mut patch.values) {
                    spatial.transform = transform.then(spatial.transform);
                }
            }
            let scale_radii = |r: &mut crate::ui::CornerRadii| {
                r.top_left *= scale;
                r.top_right *= scale;
                r.bottom_left *= scale;
                r.bottom_right *= scale;
            };
            for patch in &mut delta.clips {
                for clip in std::sync::Arc::make_mut(&mut patch.values) {
                    clip.rect = transform.transform_rect(clip.rect);
                    scale_radii(&mut clip.corner_radii);
                }
            }
            border.rect = transform.transform_rect(border.rect);
            border.view_bounds = border.rect;
            scale_radii(&mut border.corner_radii);
            for side in [
                &mut border.border.top,
                &mut border.border.right,
                &mut border.border.bottom,
                &mut border.border.left,
            ] {
                side.width *= scale;
            }
            contour = Some((
                RoundedClip::new(border.rect, border.corner_radii),
                frame_border::inner(&border),
            ));
            SoftwareRenderer
                .apply_scene_delta(&mut source, &delta)
                .unwrap();
            source.set_frame_border(Some(border));
            let mut surface = SoftwareSurface::default();
            SoftwareRenderer
                .render_composite(
                    &mut surface,
                    &[SoftwareCompositeLayer {
                        scene: &source,
                        target: RectI {
                            x: 0,
                            y: 0,
                            width: extent.width,
                            height: extent.height,
                        },
                        clip: None,
                        rounded_clips: [None; 2],
                    }],
                    extent,
                    None,
                    ColorRgba8::rgba(0, 0, 0, 0),
                )
                .unwrap();
            frames.push(surface.pixels_rgba8().to_vec());
        }
        assert_eq!(
            frames[0], frames[2],
            "hover exit did not restore the original frame"
        );
        assert_ne!(frames[0], frames[1], "hover never painted");
        let (outer, inner) = contour.unwrap();
        let mut checked = 0;
        for (index, (resting, hovered)) in frames[0]
            .chunks_exact(4)
            .zip(frames[1].chunks_exact(4))
            .enumerate()
        {
            let p = PointF {
                x: (index % extent.width as usize) as f32 + 0.5,
                y: (index / extent.width as usize) as f32 + 0.5,
            };
            if outer.coverage(p) - inner.coverage(p).min(outer.coverage(p)) > 0.001 {
                assert_eq!(
                    resting, hovered,
                    "hover recolored border at scale={scale}, pixel={p:?}"
                );
                checked += 1;
            }
        }
        assert!(checked > 100);
    }
}

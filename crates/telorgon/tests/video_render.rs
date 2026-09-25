#![cfg(all(
    target_os = "linux",
    feature = "video-linux",
    feature = "application-software"
))]
use telorgon::{
    app::*,
    graphics::{
        bridges::video_cpu::{VideoImageOptions, video_image},
        render::ImageResource,
    },
    host::application::HeadlessRuntime,
    media::video::*,
    ui::ImageId,
};
#[component(no_default)]
struct FrameView {
    #[input]
    resource: ImageResource,
}
impl Component for FrameView {
    fn view(&self) -> impl View {
        Image::resource(self.resource.clone())
            .width(2.0)
            .height(2.0)
    }
}
#[test]
fn captured_cpu_pixels_reach_the_composed_software_renderer() {
    let pixels = vec![
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
    ];
    let frame = CpuVideoFrame::packed(
        VideoFormat::rgba(2, 2, 30),
        pixels.clone(),
        FrameMetadata::default(),
    )
    .unwrap();
    let resource = video_image(&frame, ImageId(71), 4, VideoImageOptions::default()).unwrap();
    drop(frame);
    let readback = HeadlessRuntime::default()
        .run_composed_once(
            FrameView { resource },
            SizeI {
                width: 2,
                height: 2,
            },
        )
        .unwrap();
    assert_eq!(readback.pixels, pixels);
}

#[component(no_default)]
struct LiveFrameView {
    #[input]
    frames: Signal<Option<ImageResource>>,
}
impl Component for LiveFrameView {
    fn view(&self) -> impl View {
        let frame = self.watch(&self.frames);
        if let Some(resource) = frame.as_ref() {
            Image::resource(resource.clone())
                .width(2.0)
                .height(2.0)
                .into_element()
        } else {
            column().into_element()
        }
    }
}
#[test]
fn preview_revisions_replace_resources_and_removal_retires_them() {
    use telorgon::{
        foundation::MonotonicInstant, graphics::render::ImageResourceDelta,
        host::application::ComposedAppRuntime,
    };
    let frame = CpuVideoFrame::packed(
        VideoFormat::rgba(2, 2, 30),
        vec![255; 16],
        FrameMetadata::default(),
    )
    .unwrap();
    let first = video_image(&frame, ImageId(75), 1, VideoImageOptions::default()).unwrap();
    let (frames, publish) = Signal::new(Some(first));
    let mut runtime = ComposedAppRuntime::from_composed_with_extent(
        LiveFrameView { frames },
        SizeI {
            width: 2,
            height: 2,
        },
    )
    .unwrap();
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    assert!(
        runtime
            .pop_scene_delta()
            .unwrap()
            .image_resources
            .iter()
            .any(|d| matches!(d,ImageResourceDelta::Write(r) if r.content_version==1))
    );
    let next = video_image(&frame, ImageId(75), 2, VideoImageOptions::default()).unwrap();
    publish.publish(Some(next));
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    assert!(
        runtime
            .pop_scene_delta()
            .unwrap()
            .image_resources
            .iter()
            .any(|d| matches!(d,ImageResourceDelta::Write(r) if r.content_version==2))
    );
    publish.publish(None);
    runtime.prepare_frame(MonotonicInstant::ZERO, true).unwrap();
    assert!(
        runtime
            .pop_scene_delta()
            .unwrap()
            .image_resources
            .iter()
            .any(|d| matches!(d, ImageResourceDelta::Remove(ImageId(75))))
    );
}

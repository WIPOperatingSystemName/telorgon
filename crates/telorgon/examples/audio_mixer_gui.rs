//! cargo run -p telorgon --no-default-features --features desktop-audio-linux,application-software --example audio_mixer_gui
#[cfg(target_os = "linux")]
mod linux {
    use telorgon::{
        app::*,
        components::shell::AudioMixerPanel,
        host::application::audio_mixer::{AudioMixer, AudioMixerHandle},
    };
    #[component(no_default)]
    struct MixerWindow {
        #[input]
        mixer: AudioMixerHandle,
    }
    impl Component for MixerWindow {
        fn view(&self) -> impl View {
            column()
                .background(ColorRgba8::rgba(21, 25, 34, 255))
                .child(AudioMixerPanel::new(self.mixer.clone()))
        }
    }
    pub fn run() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut mixer = AudioMixer::start()?;
        let result = Application::gui("org.telorgon.examples.sound-mixer", "Sound mixer")
            .renderer(Renderer::Software)
            .window(
                Window::new("Sound mixer")
                    .size(380, 540)
                    .content(MixerWindow {
                        mixer: mixer.handle(),
                    }),
            )
            .run();
        mixer.shutdown();
        result?;
        Ok(())
    }
}
#[cfg(target_os = "linux")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    linux::run()
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}

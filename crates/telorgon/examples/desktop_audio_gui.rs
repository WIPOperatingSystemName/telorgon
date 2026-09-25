//! cargo run -p telorgon --no-default-features --features desktop-audio-linux,application-software --example desktop_audio_gui
//! Desktop authority is explicit. Opening this example only observes; device changes
//! require button presses. No recording, metering capture or audio playback is opened.
#[cfg(target_os = "linux")]
fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    use telorgon::{
        app::*,
        components::shell::AudioSettings,
        host::application::desktop_audio::DesktopAudio,
        integrations::pipewire::{Connection, ConnectionConfig, Remote},
    };
    let mut connection = Connection::connect(ConnectionConfig::default(), Remote::Default)?;
    let mut audio = DesktopAudio::start(connection.handle())?;
    let result = Application::gui("org.telorgon.examples.desktop-audio", "Desktop audio")
        .renderer(Renderer::Software)
        .window(
            Window::new("Desktop audio")
                .size(1000, 680)
                .content(AudioSettings::new(audio.handle())),
        )
        .run();
    audio.shutdown();
    connection.shutdown()?;
    result?;
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("This example requires Linux.");
}

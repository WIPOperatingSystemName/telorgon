//! A background Telorgon app. Its tray menu toggles activity and exits the process.
//! cargo run -p telorgon --no-default-features --features tray-linux --example tray_publisher
use telorgon::tray::*;
fn menu(paused: bool) -> TrayMenu {
    TrayMenu::new(vec![
        TrayMenuItem::action(1, "Pause activity").checked(paused),
        TrayMenuItem::action(2, "Options")
            .submenu(vec![TrayMenuItem::action(3, "Request attention")]),
        TrayMenuItem::separator(4),
        TrayMenuItem::action(5, "Quit"),
    ])
    .unwrap()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut icon = TrayIcon::publish(
        TrayIconConfig::new("org.telorgon.TrayExample")
            .title("Telorgon background app")
            .icon(TrayImage::named("folder-sync"))
            .tooltip("Activity running")
            .menu(menu(false)),
    )?;
    let mut paused = false;
    loop {
        match icon.try_next_event() {
            Some(TrayEvent::MenuSelected { item: 1 }) => {
                paused = !paused;
                icon.set_menu(menu(paused))?;
                icon.set_tooltip(if paused {
                    "Activity paused"
                } else {
                    "Activity running"
                });
            }
            Some(TrayEvent::MenuSelected { item: 3 }) => {
                icon.set_status(TrayStatus::NeedsAttention)
            }
            Some(TrayEvent::MenuSelected { item: 5 }) => break,
            Some(TrayEvent::Activated { .. }) => {
                icon.set_status(TrayStatus::Active);
                println!("Tray icon activated");
            }
            Some(event) => println!("{event:?}"),
            None => std::thread::sleep(std::time::Duration::from_millis(20)),
        }
    }
    icon.shutdown();
    Ok(())
}

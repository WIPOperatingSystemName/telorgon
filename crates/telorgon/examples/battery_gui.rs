//! cargo run -p telorgon --example battery_gui
use battery::{BatteryAvailability, BatteryMonitorHandle};
use telorgon::app::*;

const TEXT: ColorRgba8 = ColorRgba8::rgba(242, 242, 244, 255);

#[component(no_default)]
struct BatteryIndicator {
    #[input]
    battery: BatteryMonitorHandle,
}

impl Component for BatteryIndicator {
    fn view(&self) -> impl View {
        let state = self.watch(&self.battery.signal());
        let mut content = column()
            .padding(24.0)
            .gap(12.0)
            .background(ColorRgba8::rgba(30, 30, 32, 255))
            .child(text("Battery").size(24.0).color(TEXT));
        if state.availability != BatteryAvailability::Ready {
            content = content.child(
                text(format!(
                    "{:?}: showing last known readings",
                    state.availability
                ))
                .size(14.0)
                .color(TEXT),
            );
        }
        if state.batteries.is_empty() {
            content = content.child(text("No batteries discovered").color(TEXT));
        }
        for battery in &state.batteries {
            let percentage = battery
                .status
                .percentage
                .map_or("Unavailable".into(), |v| format!("{v}%"));
            content = content.child(
                text(format!(
                    "{}: {} · {:?}",
                    battery.model.as_deref().unwrap_or(&battery.id),
                    percentage,
                    battery.status.state
                ))
                .size(16.0)
                .color(TEXT),
            );
        }
        content
    }
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let monitor =
        futures_lite::future::block_on(battery::monitor(battery::BatteryMonitorConfig::default()))?;
    let result = Application::gui("org.telorgon.example.battery", "Battery")
        .window(
            Window::new("Battery")
                .size(520, 360)
                .content(BatteryIndicator {
                    battery: monitor.handle(),
                }),
        )
        .run();
    futures_lite::future::block_on(monitor.shutdown())?;
    result?;
    Ok(())
}

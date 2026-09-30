//! cargo run -p telorgon --no-default-features --example battery
//! Add -- --watch to observe transitions until interrupted.
use telorgon::battery;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    futures_lite::future::block_on(run())
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--watch") {
        let monitor = battery::monitor(battery::BatteryMonitorConfig::default()).await?;
        let handle = monitor.handle();
        let mut events = handle.subscribe()?;
        println!("Initial state: {:?}", &*handle.state());
        while let Some(event) = events.next().await {
            if event.started_charging() {
                println!("Charging started");
            }
            if event.stopped_charging() {
                println!("Charging stopped");
            }
            if event == battery::BatteryEvent::ResyncRequired {
                println!("Resynchronized: {:?}", &*handle.state());
            } else {
                println!("{event:?}");
            }
        }
        monitor.shutdown().await?;
        return Ok(());
    }
    let snapshot = battery::snapshot().await?;
    println!("External power: {:?}", snapshot.external_power);
    for battery in snapshot.batteries {
        println!(
            "{} ({:?}, {:?}): {:?}% {:?}",
            battery.id, battery.kind, battery.scope, battery.charge_percent, battery.state
        );
        println!(
            "  Capacity retention: {:?}% · cycles: {:?} · temperature: {:?} °C",
            battery.capacity_health_percent, battery.cycle_count, battery.temperature_celsius
        );
        println!(
            "  Power: {:?} W · empty in: {:?} · full in: {:?}",
            battery.power_watts, battery.time_to_empty, battery.time_to_full
        );
    }
    Ok(())
}

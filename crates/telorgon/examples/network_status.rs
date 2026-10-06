//! Read-only inspection. NetworkController::start starts observation, not a connection.
use std::time::{Duration, Instant};
use telorgon::network::{NetworkController, NetworkServiceState};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut network = NetworkController::new(Default::default())?;
    let observer = network.observer();
    network.start()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let state = observer.signal().snapshot();
        match state.state {
            NetworkServiceState::Ready => {
                println!("{:?}: {:?}", state.backend, state.connectivity);
                for interface in &state.interfaces {
                    println!(
                        "{}: {:?}, {:?}",
                        interface.name, interface.kind, interface.state
                    );
                    for address in interface
                        .ipv4
                        .addresses
                        .iter()
                        .chain(&interface.ipv6.addresses)
                    {
                        println!("  {}/{}", address.address, address.prefix);
                    }
                    for ap in &interface.access_points {
                        println!(
                            "  {}: {}%, {:?}",
                            ap.ssid
                                .as_ref()
                                .map(|s| s.display_name())
                                .unwrap_or_else(|| "<hidden>".into()),
                            ap.strength,
                            ap.security
                        );
                    }
                }
                break;
            }
            NetworkServiceState::Unavailable
            | NetworkServiceState::Restricted
            | NetworkServiceState::Stopped => {
                return Err(state
                    .last_error
                    .clone()
                    .unwrap_or(telorgon::network::NetworkError::Unavailable)
                    .into());
            }
            _ if Instant::now() >= deadline => {
                return Err(telorgon::network::NetworkError::TimedOut.into());
            }
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
    network.shutdown()?;
    Ok(())
}

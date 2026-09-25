//! cargo run -p telorgon --example settings -- /tmp/settings.toml [--autosave]
use std::{path::PathBuf, time::Duration};
use telorgon::{
    data::{Autosave, EntrySpec, Registry, Settings},
    foundation::{ColorRgba8, SizeI},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().unwrap_or_else(|| "settings.toml".into()));
    let autosave = args.any(|arg| arg == "--autosave");
    let registry = Registry::new("settings-example", 1);
    let volume = registry.register(EntrySpec::new("audio.volume", 0.8_f32).validate(|value| {
        if value.is_finite() && (0.0..=1.0).contains(value) {
            Ok(())
        } else {
            Err("volume must be between zero and one".into())
        }
    }))?;
    let accent = registry.create("appearance.accent", ColorRgba8::rgba(90, 140, 255, 255))?;
    registry.create(
        "panel.size",
        SizeI {
            width: 400,
            height: 48,
        },
    )?;
    registry.load_if_exists(&path)?;
    if autosave {
        registry.enable_autosave(
            Autosave::to(&path)
                .debounce(Duration::from_millis(500))
                .max_delay(Duration::from_secs(5)),
        )?;
    }
    Settings::install_global(registry)?;
    let settings = Settings::global()?;
    let changes = settings.subscribe()?;

    // Typical application code: handles stay valid across transactions and reloads.
    settings.transaction(|tx| {
        tx.set(&volume, 0.6)?;
        tx.set(&accent, ColorRgba8::rgba(255, 120, 80, 255))
    })?;
    if let Some(change) = changes.try_recv()? {
        println!(
            "Settings revision {} changed {:?}",
            change.revision, change.keys
        );
    }
    println!("Volume: {}", settings.get::<f32>("audio.volume")?.get()?);
    if autosave {
        // During normal operation the worker saves changes automatically. At application exit:
        settings.shutdown()?;
    } else {
        settings.save(&path)?;
    }
    Ok(())
}

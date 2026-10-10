use super::*;
use crate::ImageId;
use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::{PermissionsExt, symlink},
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telorgon-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn store(&self) -> SettingsStore {
        SettingsStore::new("settings-test", self.0.join("settings.toml"))
    }
    fn photo(&self) -> PathBuf {
        let path = self.0.join("source.bmp");
        image::RgbaImage::from_pixel(640, 400, image::Rgba([12, 34, 56, 255]))
            .save(&path)
            .unwrap();
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn missing_preferences_have_defaults_without_creating_files() {
    let fixture = Fixture::new();
    assert_eq!(fixture.store().load().unwrap(), Preferences::default());
    assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 0);
}

#[test]
fn partial_updates_preserve_other_preferences_and_unknown_entries() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let mut sound = SoundSettings::default();
    sound.output.device = "speakers".into();
    sound.output.volume = Some(0.25);
    store.save_sound(&sound).unwrap();
    let mut text = fs::read_to_string(&store.path).unwrap();
    text.push_str("\n[values.future]\nkeep = true\n");
    fs::write(&store.path, text).unwrap();
    let display = DisplayConfiguration {
        scale: Some(1.5),
        ..Default::default()
    };
    store.save_display(&display).unwrap();
    let reopened = fixture.store().load().unwrap();
    assert_eq!(reopened.sound, sound);
    assert_eq!(reopened.display, display);
    assert!(
        fs::read_to_string(&store.path)
            .unwrap()
            .contains("keep = true")
    );
    assert_eq!(
        fs::metadata(&store.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[test]
fn invalid_or_incompatible_files_are_not_overwritten() {
    let fixture = Fixture::new();
    let store = fixture.store();
    store.save_sound(&SoundSettings::default()).unwrap();
    let valid = fs::read_to_string(&store.path).unwrap();
    for invalid in [
        "not toml".into(),
        valid.replace("version = 1", "version = 9"),
        valid.replace("settings-test", "another-app"),
    ] {
        fs::write(&store.path, &invalid).unwrap();
        assert!(store.load().is_err());
        assert!(
            store
                .save_display(&DisplayConfiguration::default())
                .is_err()
        );
        assert_eq!(fs::read_to_string(&store.path).unwrap(), invalid);
    }
    fs::write(&store.path, &valid).unwrap();
    let mut sound = SoundSettings::default();
    sound.output.volume = Some(f32::NAN);
    assert!(store.save_sound(&sound).is_err());
    assert_eq!(fs::read_to_string(&store.path).unwrap(), valid);
}

#[test]
fn independent_writers_keep_both_sections() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let left = store.clone();
    let right = store.clone();
    let gate = Arc::new(std::sync::Barrier::new(2));
    let other_gate = gate.clone();
    let a = std::thread::spawn(move || {
        gate.wait();
        for _ in 0..30 {
            left.save_display(&DisplayConfiguration {
                scale: Some(2.0),
                ..Default::default()
            })
            .unwrap();
        }
    });
    let b = std::thread::spawn(move || {
        other_gate.wait();
        for _ in 0..30 {
            right
                .save_sound(&SoundSettings {
                    input: SoundChannel {
                        device: "microphone".into(),
                        muted: Some(true),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .unwrap();
        }
    });
    a.join().unwrap();
    b.join().unwrap();
    let saved = fixture.store().load().unwrap();
    assert_eq!(saved.display.scale, Some(2.0));
    assert_eq!(saved.sound.input.device, "microphone");
    assert_eq!(saved.sound.input.muted, Some(true));
}

#[test]
#[ignore = "helper invoked in separate processes by process_writers_keep_both_sections"]
fn settings_writer_process() {
    let path = std::env::var_os("TELORGON_SETTINGS_TEST_PATH")
        .expect("parent supplies a private settings path");
    let section = std::env::var("TELORGON_SETTINGS_TEST_SECTION").unwrap();
    let store = SettingsStore::new("settings-test", PathBuf::from(path));
    for _ in 0..30 {
        match section.as_str() {
            "display" => store
                .save_display(&DisplayConfiguration {
                    scale: Some(2.0),
                    ..Default::default()
                })
                .unwrap(),
            "sound" => store
                .save_sound(&SoundSettings {
                    input: SoundChannel {
                        device: "microphone".into(),
                        muted: Some(true),
                        ..Default::default()
                    },
                    ..Default::default()
                })
                .unwrap(),
            _ => panic!("invalid writer section"),
        }
    }
}

#[test]
fn process_writers_keep_both_sections() {
    let fixture = Fixture::new();
    let mut children: Vec<_> = ["display", "sound"]
        .into_iter()
        .map(|section| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "services::desktop_settings::tests::settings_writer_process",
                    "--ignored",
                ])
                .env("TELORGON_SETTINGS_TEST_PATH", &fixture.store().path)
                .env("TELORGON_SETTINGS_TEST_SECTION", section)
                .stdout(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    for child in &mut children {
        assert!(child.wait().unwrap().success());
    }
    let saved = fixture.store().load().unwrap();
    assert_eq!(saved.display.scale, Some(2.0));
    assert_eq!(saved.sound.input.device, "microphone");
    assert_eq!(saved.sound.input.muted, Some(true));
}

#[test]
fn import_normalizes_images_and_thumbnail_selection_retains_verified_pixels() {
    let fixture = Fixture::new();
    let store = fixture.store();
    let source = fixture.photo();
    let imported = store
        .import_background_with_thumbnail(&source, ImageId(12))
        .unwrap();
    assert_eq!(imported.image.image, ImageId(12));
    assert!(imported.image.extent.width <= 320 && imported.image.extent.height <= 180);
    let settings = imported.validated.settings().clone();
    assert_eq!(store.import_background(&source).unwrap(), settings);
    fs::remove_file(source).unwrap();
    store
        .save_personalization_validated(&imported.validated)
        .unwrap();
    assert_eq!(store.load_personalization().unwrap(), settings);
    assert_eq!(store.list_backgrounds().unwrap().len(), 1);
    let (image, validated) = store
        .load_background_validated(&settings, ImageId(99))
        .unwrap();
    assert_eq!(
        image.unwrap().extent,
        crate::SizeI {
            width: 640,
            height: 400
        }
    );
    validated.unwrap().verify(&store).unwrap();
    assert!(store.delete_background(&settings).is_err());
    store
        .save_personalization(&PersonalizationSettings::default())
        .unwrap();
    store.delete_background(&settings).unwrap();
    assert!(imported.validated.verify(&store).is_err());
    assert!(store.list_backgrounds().unwrap().is_empty());
}

#[test]
fn changed_files_and_wrong_libraries_cannot_reuse_validation() {
    let fixture = Fixture::new();
    let other = Fixture::new();
    let store = fixture.store();
    let imported = store
        .import_background_with_thumbnail(&fixture.photo(), ImageId(1))
        .unwrap();
    let settings = imported.validated.settings().clone();
    other.store().import_background(&other.photo()).unwrap();
    assert!(imported.validated.verify(&other.store()).is_err());
    let path = store.background_path(&settings).unwrap().unwrap();
    let bytes = fs::read(&path).unwrap();
    let original_time = fs::metadata(&path).unwrap().modified().unwrap();
    let mut changed = bytes.clone();
    changed[20] ^= 1;
    fs::write(&path, changed).unwrap();
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(original_time)
        .unwrap();
    assert!(imported.validated.verify(&store).is_err());
    assert!(
        store
            .save_personalization_validated(&imported.validated)
            .is_err()
    );
    assert!(store.list_backgrounds().unwrap()[0].error.is_some());
    fs::write(path, bytes).unwrap();
    imported.validated.verify(&store).unwrap();
}

#[test]
fn managed_paths_reject_traversal_symlinks_and_nonregular_files() {
    let fixture = Fixture::new();
    let store = fixture.store();
    for background in [
        "../outside.png",
        "/tmp/outside.png",
        "background-deadbeef.png",
    ] {
        assert!(
            store
                .background_path(&PersonalizationSettings {
                    background: Some(background.into())
                })
                .is_err()
        );
    }
    let target = fixture.0.join("outside");
    fs::write(&target, b"keep me").unwrap();
    symlink(&target, &store.path).unwrap();
    assert!(store.load().is_err());
    assert!(store.save_sound(&SoundSettings::default()).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"keep me");
    fs::remove_file(&store.path).unwrap();
    let directory = store.background_directory().unwrap();
    symlink(&fixture.0, &directory).unwrap();
    assert!(store.ensure_background_directory().is_err());
    fs::remove_file(directory).unwrap();
    let settings = store.import_background(&fixture.photo()).unwrap();
    let path = store.background_path(&settings).unwrap().unwrap();
    fs::remove_file(&path).unwrap();
    symlink(&target, &path).unwrap();
    assert!(
        store
            .load_background_validated(&settings, ImageId(1))
            .is_err()
    );
    assert!(store.delete_background(&settings).is_err());
    assert_eq!(fs::read(&target).unwrap(), b"keep me");
}

#[test]
fn corrupt_images_and_large_inputs_are_rejected_without_selecting_them() {
    let fixture = Fixture::new();
    let path = fixture.0.join("invalid.png");
    fs::write(&path, b"not an image").unwrap();
    assert!(fixture.store().import_background(&path).is_err());
    let file = fs::File::create(&path).unwrap();
    file.set_len(65 * 1024 * 1024).unwrap();
    assert!(fixture.store().import_background(&path).is_err());
    assert_eq!(fixture.store().load().unwrap(), Preferences::default());
}

#[test]
fn protocol_checks_versions_bounds_and_preferences() {
    let display = DisplayConfiguration {
        scale: Some(1.5),
        ..Default::default()
    };
    let encoded = protocol::encode(&display).unwrap();
    assert_eq!(
        protocol::decode::<DisplayConfiguration>(&encoded).unwrap(),
        display
    );
    assert!(
        protocol::decode::<DisplayConfiguration>(
            &encoded.replace("\"version\":1", "\"version\":2")
        )
        .is_err()
    );
    assert!(protocol::decode::<DisplayConfiguration>(&encoded.replace("1.5", "0.5")).is_err());
    assert!(
        protocol::decode::<DisplayConfiguration>(&" ".repeat(protocol::MAX_MESSAGE_BYTES + 1))
            .is_err()
    );
    assert!(
        protocol::decode::<DisplayConfiguration>(
            r#"{"version":1,"payload":{"scale":1,"unexpected":true}}"#
        )
        .is_err()
    );
    let mut snapshot = ShellSnapshot {
        personalization: Some(PersonalizationSnapshot::default()),
        ..Default::default()
    };
    assert_eq!(
        protocol::decode::<ShellSnapshot>(&protocol::encode_snapshot(&snapshot).unwrap()).unwrap(),
        snapshot
    );
    snapshot.display.resolved_scale = f32::NAN;
    assert!(protocol::encode_snapshot(&snapshot).is_err());
    assert!(SettingsEndpoint::new(":1.2", "/settings", "org.telorgon.Settings1").is_err());
    assert!(
        SettingsEndpoint::new(
            "org.telorgon.Settings",
            "relative",
            "org.telorgon.Settings1"
        )
        .is_err()
    );
}

struct PrivateBus(Child);
impl Drop for PrivateBus {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Clone)]
struct Endpoint {
    state: Arc<Mutex<ShellSnapshot>>,
    store: SettingsStore,
}
#[zbus::interface(name = "org.telorgon.SettingsTest1")]
impl Endpoint {
    fn snapshot(&self) -> zbus::fdo::Result<String> {
        protocol::encode_snapshot(&self.state.lock().unwrap()).map_err(zbus::fdo::Error::Failed)
    }
    fn reload_settings(&self) -> zbus::fdo::Result<()> {
        self.store
            .load()
            .map(|_| ())
            .map_err(zbus::fdo::Error::Failed)
    }
    fn apply_personalization(&self, configuration: &str) -> zbus::fdo::Result<()> {
        let settings = protocol::decode(configuration).map_err(zbus::fdo::Error::Failed)?;
        self.state.lock().unwrap().personalization = Some(PersonalizationSnapshot {
            current: settings,
            error: None,
        });
        Ok(())
    }
    fn delete_background(&self, configuration: &str) -> zbus::fdo::Result<()> {
        let settings = protocol::decode(configuration).map_err(zbus::fdo::Error::Failed)?;
        self.store
            .delete_background(&settings)
            .map_err(zbus::fdo::Error::Failed)
    }
    fn preview_display(&self, configuration: &str) -> zbus::fdo::Result<u64> {
        let configuration = protocol::decode(configuration).map_err(zbus::fdo::Error::Failed)?;
        let mut state = self.state.lock().unwrap();
        state.display.current = configuration;
        state.display.preview = Some(77);
        Ok(77)
    }
    fn confirm_display(&self, token: u64) -> zbus::fdo::Result<()> {
        let mut state = self.state.lock().unwrap();
        if state.display.preview != Some(token) {
            return Err(zbus::fdo::Error::Failed("Expired preview".into()));
        }
        state.display.preview = None;
        Ok(())
    }
    fn revert_display(&self, token: u64) -> zbus::fdo::Result<()> {
        self.confirm_display(token)
    }
}

#[test]
fn client_calls_the_shell_protocol_over_a_private_bus() {
    let fixture = Fixture::new();
    let config = fixture.0.join("bus.conf");
    fs::write(&config, format!(r#"<busconfig><type>session</type><listen>unix:path={}/bus</listen><auth>EXTERNAL</auth><policy context="default"><allow send_destination="*"/><allow receive_sender="*"/><allow own="*"/></policy></busconfig>"#, fixture.0.display())).unwrap();
    let mut bus = PrivateBus(
        Command::new("dbus-daemon")
            .arg("--nofork")
            .arg("--print-address=1")
            .arg(format!("--config-file={}", config.display()))
            .stdout(Stdio::piped())
            .spawn()
            .expect("private test requires dbus-daemon"),
    );
    let mut address = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    assert!(!address.is_empty(), "private bus failed to start");
    let endpoint = SettingsEndpoint::new(
        "org.telorgon.SettingsTest",
        "/org/telorgon/SettingsTest",
        "org.telorgon.SettingsTest1",
    )
    .unwrap();
    let state = Arc::new(Mutex::new(ShellSnapshot::default()));
    let _server = zbus::blocking::connection::Builder::address(address.trim())
        .unwrap()
        .name(endpoint.bus_name())
        .unwrap()
        .serve_at(
            endpoint.object_path(),
            Endpoint {
                state,
                store: fixture.store(),
            },
        )
        .unwrap()
        .build()
        .unwrap();
    let connection = zbus::blocking::connection::Builder::address(address.trim())
        .unwrap()
        .method_timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let client = SettingsClient::from_connection(connection, endpoint);
    client.reload().unwrap();
    let display = DisplayConfiguration {
        scale: Some(1.5),
        ..Default::default()
    };
    assert_eq!(client.preview_display(&display).unwrap(), 77);
    assert_eq!(client.snapshot().unwrap().display.current, display);
    assert!(
        client
            .confirm_display(99)
            .unwrap_err()
            .contains("Expired preview")
    );
    client.confirm_display(77).unwrap();
    assert_eq!(client.snapshot().unwrap().display.preview, None);
    client.preview_display(&display).unwrap();
    client.revert_display(77).unwrap();
    let settings = fixture.store().import_background(&fixture.photo()).unwrap();
    client.apply_personalization(&settings).unwrap();
    assert_eq!(
        client.snapshot().unwrap().personalization.unwrap().current,
        settings
    );
    client.delete_background(&settings).unwrap();
    assert!(fixture.store().list_backgrounds().unwrap().is_empty());
}

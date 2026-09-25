use super::*;
use crate::services::session::{self, DesktopSettings, Environment, SessionConfig, SessionOwner};
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
#[ignore = "requires installed gtk-query-settings; private headless protocol diagnostic"]
fn managed_session_gtk_decoration_layout() {
    check_layout(None, "menu:minimize,maximize,close");
    check_layout(Some(":close"), ":close");
}

fn check_layout(user_layout: Option<&str>, expected: &str) {
    let runtime = std::env::temp_dir().join(format!("telorgon-gtk-probe-{}", std::process::id()));
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let display = Display::new().unwrap();
    let socket = display
        .add_socket_in(&runtime, Some("wayland-test"))
        .unwrap();
    let _native = NativeCompositor::new(&display, ClientLimits::default()).unwrap();
    let mut env = Environment::inherited().desktop(&runtime, &socket, "telorgon-test-shell");
    // Do not contact or activate any real desktop services during this diagnostic.
    env.0.insert(
        "DBUS_SESSION_BUS_ADDRESS".into(),
        "unix:path=/nonexistent/telorgon-test-bus".into(),
    );
    env.0.insert("GSETTINGS_BACKEND".into(), "dconf".into());
    env.0.insert(
        "XDG_CONFIG_HOME".into(),
        runtime.join("config").into_os_string(),
    );
    // Make the inherited profile deterministic, with a real private user DB to verify
    // that session defaults do not overwrite explicitly saved preferences.
    let profile = runtime.join("inherited-profile");
    std::fs::write(&profile, "user-db:user\n").unwrap();
    env.0
        .insert("DCONF_PROFILE".into(), profile.into_os_string());
    if let Some(layout) = user_layout {
        let sources = runtime.join("user-settings");
        std::fs::create_dir(&sources).unwrap();
        std::fs::write(
            sources.join("buttons"),
            format!("[org/gnome/desktop/wm/preferences]\nbutton-layout='{layout}'\n"),
        )
        .unwrap();
        let database = runtime.join("config/dconf/user");
        std::fs::create_dir_all(database.parent().unwrap()).unwrap();
        assert!(
            Command::new("dconf")
                .arg("compile")
                .arg(database)
                .arg(sources)
                .status()
                .unwrap()
                .success()
        );
    }
    let config = SessionConfig {
        recovery: false,
        ..SessionConfig::new("gtk-probe")
    }
    .desktop_settings(DesktopSettings::default());
    let owner = SessionOwner::start(env, config).unwrap();
    let mut command = Command::new("gtk-query-settings");
    command
        .arg("gtk-decoration-layout")
        .env("GDK_BACKEND", "wayland")
        .env_remove("GTK_MODULES")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = session::current()
        .unwrap()
        .spawn_helper(&mut command)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() && Instant::now() < deadline {
        display
            .dispatch_and_flush(Some(Duration::from_millis(10)))
            .unwrap();
    }
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    owner.close();
    drop(owner);
    drop(_native);
    drop(display);
    std::fs::remove_dir_all(runtime).unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}\n{stderr}");
    assert!(
        stdout.contains(&format!("\"{expected}\"")),
        "{stdout}\n{stderr}"
    );
}

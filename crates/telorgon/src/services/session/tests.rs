use super::*;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::future::Future;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

// Only these fixtures publish the process singleton; all are serialized without changing env vars.
static OWNER_TEST: Mutex<()> = Mutex::new(());
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telorgon-session-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        Self(path)
    }
    fn config(&self) -> SessionConfig {
        SessionConfig {
            recovery_directory: Some(self.0.join("recovery")),
            shutdown_timeout: Duration::from_millis(500),
            ..SessionConfig::default()
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct TestWake;
impl Wake for TestWake {
    fn wake(self: Arc<Self>) {}
}
fn wait_until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < Duration::from_secs(15), "timed out");
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn wait<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(TestWake));
    let start = Instant::now();
    loop {
        if let Poll::Ready(value) = future.as_mut().poll(&mut Context::from_waker(&waker)) {
            return value;
        }
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "future timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn shell_environment_replaces_only_session_keys_without_mutating_the_parent() {
    let original = Environment(BTreeMap::from([
        ("PATH".into(), "/usr/bin".into()),
        ("DISPLAY".into(), ":1".into()),
        ("WAYLAND_DISPLAY".into(), "old-0".into()),
        ("WAYLAND_SOCKET".into(), "123".into()),
        ("XDG_ACTIVATION_TOKEN".into(), "one-shot".into()),
        ("LANG".into(), "en_US.UTF-8".into()),
    ]));
    let env = original.desktop(
        std::path::Path::new("/not-a-real-runtime"),
        "wayland-7",
        "myde",
    );
    assert_eq!(
        env.get("WAYLAND_DISPLAY"),
        Some(std::ffi::OsStr::new("wayland-7"))
    );
    assert_eq!(
        env.get("XDG_CURRENT_DESKTOP"),
        Some(std::ffi::OsStr::new("myde"))
    );
    assert!(env.get("DISPLAY").is_none());
    assert!(env.get("WAYLAND_SOCKET").is_none());
    assert!(env.get("XDG_ACTIVATION_TOKEN").is_none());
    assert_eq!(env.get("PATH"), original.get("PATH"));
    assert_eq!(env.get("LANG"), original.get("LANG"));
    assert_eq!(original.get("DISPLAY"), Some(std::ffi::OsStr::new(":1")));
}

#[test]
#[cfg(target_os = "linux")]
fn runtime_directory_rejects_symlinks_and_insecure_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let fixture = Fixture::new();
    let env_for = |path: &std::path::Path| {
        Environment(BTreeMap::from([(
            "XDG_RUNTIME_DIR".into(),
            path.as_os_str().into(),
        )]))
    };
    assert_eq!(env_for(&fixture.0).runtime_directory().unwrap(), fixture.0);
    let alias = fixture.0.join("alias");
    symlink(&fixture.0, &alias).unwrap();
    assert!(env_for(&alias).runtime_directory().is_err());
    std::fs::set_permissions(&fixture.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(env_for(&fixture.0).runtime_directory().is_err());
}

#[test]
#[cfg(all(target_os = "linux", feature = "shell-wayland-linux"))]
fn wayland_socket_auto_selection_and_cleanup_use_the_explicit_directory() {
    let fixture = Fixture::new();
    let old_env = std::env::var_os("XDG_RUNTIME_DIR");
    let first = crate::integrations::wayland::server::Display::new().unwrap();
    let second = crate::integrations::wayland::server::Display::new().unwrap();
    assert_eq!(first.add_socket_in(&fixture.0, None).unwrap(), "wayland-0");
    assert_eq!(second.add_socket_in(&fixture.0, None).unwrap(), "wayland-1");
    assert!(fixture.0.join("wayland-0").exists());
    assert!(second.add_socket_in(&fixture.0, Some("../escape")).is_err());
    assert_eq!(std::env::var_os("XDG_RUNTIME_DIR"), old_env);
    drop(first);
    assert!(!fixture.0.join("wayland-0").exists());
    let third = crate::integrations::wayland::server::Display::new().unwrap();
    assert_eq!(third.add_socket_in(&fixture.0, None).unwrap(), "wayland-0");
}

#[test]
#[cfg(target_os = "linux")]
fn singleton_lifetime_environments_and_literal_arguments() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let before = command("/bin/sh").arg("-c").arg("exit 0");
    assert!(matches!(before.clone().spawn(), Err(Error::NotReady)));
    let mut env = Environment::inherited();
    env.0.insert("TELORGON_FIXTURE".into(), "inherited".into());
    let owner = SessionOwner::start(env, fixture.config()).unwrap();
    assert!(matches!(
        SessionOwner::start(Environment::inherited(), fixture.config()),
        Err(Error::AlreadyRunning)
    ));
    let handle = current().unwrap();
    let out = wait(
        command("/bin/sh")
            .args([
                "-c",
                "printf '%s|%s|%s' \"$TELORGON_FIXTURE\" \"$1\" \"$PWD\"",
                "test",
                "$(echo never); *",
            ])
            .env("TELORGON_FIXTURE", "override")
            .current_dir(&fixture.0)
            .output(),
    )
    .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("override|$(echo never); *|{}", fixture.0.display())
    );
    let removed = wait(
        command("/bin/sh")
            .args(["-c", "printf '%s' \"${TELORGON_FIXTURE-unset}\""])
            .env_remove("TELORGON_FIXTURE")
            .output(),
    )
    .unwrap();
    assert_eq!(removed.stdout, b"unset");
    assert!(wait(before.status()).unwrap().success());
    assert!(matches!(
        command("/definitely/missing/program").spawn(),
        Err(Error::Io(_))
    ));
    owner.close();
    drop(owner);
    assert!(matches!(current(), Err(Error::NotReady)));
    assert!(matches!(
        handle.command("/bin/true").spawn(),
        Err(Error::Closing)
    ));
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    assert_eq!(current().unwrap().phase(), SessionPhase::Ready);
    next.close();
}

#[test]
#[cfg(target_os = "linux")]
fn captures_both_pipes_and_bounds_noisy_output() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let out = wait(command("/bin/sh").args(["-c", "i=0; while [ \"$i\" -lt 4096 ]; do printf 12345678; printf abcdefgh >&2; i=$((i+1)); done"])
        .output_limit(137).output()).unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout.len(), 137);
    assert_eq!(out.stderr.len(), 137);
    assert!(out.stdout_truncated && out.stderr_truncated);
    let input = vec![b'x'; 256 * 1024];
    let out = wait(command("/bin/cat").input(input.clone()).output()).unwrap();
    assert_eq!(out.stdout, input);
    assert!(out.status.success());
    owner.close();
}

#[test]
fn cancelling_child_waits_releases_registrations_and_wakes_only_live_waiters() {
    let completion = Arc::new(command::Completion::new(1));
    let child = ManagedChild {
        completion: completion.clone(),
    };
    let waker = Waker::from(Arc::new(TestWake));
    for _ in 0..100 {
        let future = child.status();
        let mut future = std::pin::pin!(future);
        assert!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
    }
    completion.finish(Err(Error::Closing));
    assert!(matches!(wait(child.status()), Err(Error::Closing)));
}

#[test]
#[cfg(target_os = "linux")]
fn dropped_handles_are_reaped_and_normal_termination_is_not_a_crash() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    drop(command("/bin/true").spawn().unwrap());
    let child = command("/bin/sleep")
        .arg("10")
        .recover(true)
        .spawn()
        .unwrap();
    owner.quiesce();
    assert!(matches!(command("/bin/true").spawn(), Err(Error::Closing)));
    owner.resume();
    assert_eq!(current().unwrap().phase(), SessionPhase::Ready);
    owner.close();
    drop(owner);
    assert!(wait(child.status()).is_ok());
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    assert!(
        pending_recovery().unwrap().is_empty(),
        "normal SIGTERM shutdown must not offer crash recovery"
    );
    next.close();
}

#[test]
#[cfg(target_os = "linux")]
fn crashed_apps_retry_with_a_limit_and_are_offered_after_restart() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let log = fixture.0.join("attempts");
    let child = command("/bin/sh")
        .args([
            OsString::from("-c"),
            OsString::from("printf x >> \"$1\"; exit 1"),
            OsString::from("test"),
            log.as_os_str().into(),
        ])
        .restart(RestartPolicy::OnFailure)
        .recover(true)
        .spawn()
        .unwrap();
    assert!(!wait(child.status()).unwrap().success());
    assert_eq!(std::fs::read(&log).unwrap(), b"xxxx");
    assert_eq!(pending_recovery().unwrap().len(), 1);
    owner.close();
    drop(owner);
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let pending = pending_recovery().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(!pending[0].original_process_alive());
    current()
        .unwrap()
        .dismiss_recovery(pending[0].id())
        .unwrap();
    assert!(
        pending[0].restore().is_err(),
        "dismissed entries cannot launch again"
    );
    next.close();
}

#[test]
#[cfg(target_os = "linux")]
fn unclean_host_run_preserves_live_process_identity_without_duplicate_relaunch() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let child = command("/bin/sleep")
        .arg("0.5")
        .recover(true)
        .spawn()
        .unwrap();
    owner.abort();
    drop(owner);
    assert!(matches!(wait(child.status()), Err(Error::Closing)));
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let pending = pending_recovery().unwrap();
    assert_eq!(pending.len(), 1);
    if pending[0].original_process_alive() {
        assert!(pending[0].restore().is_err());
    }
    wait_until(|| !pending[0].original_process_alive());
    let restored = pending[0].restore().unwrap();
    assert!(pending_recovery().unwrap().is_empty());
    assert!(wait(restored.status()).unwrap().success());
    next.close();
}

#[test]
#[cfg(target_os = "linux")]
fn application_lookup_respects_user_precedence_and_launches_with_session_env() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let user = fixture.0.join("user/applications");
    let system = fixture.0.join("system/applications");
    std::fs::create_dir_all(&user).unwrap();
    std::fs::create_dir_all(&system).unwrap();
    std::fs::write(
        system.join("fixture.desktop"),
        "[Desktop Entry]\nType=Application\nName=System\nExec=/bin/false\n",
    )
    .unwrap();
    std::fs::write(
        user.join("fixture.desktop"),
        "[Desktop Entry]\nType=Application\nName=User\nExec=/bin/true %U\n",
    )
    .unwrap();
    let mut env = Environment::inherited();
    env.0.insert(
        "XDG_DATA_HOME".into(),
        fixture.0.join("user").into_os_string(),
    );
    env.0.insert(
        "XDG_DATA_DIRS".into(),
        fixture.0.join("system").into_os_string(),
    );
    let owner = SessionOwner::start(env, fixture.config()).unwrap();
    let launch = wait(
        application("fixture.desktop")
            .open_url("https://example.com/")
            .launch(),
    )
    .unwrap();
    assert_eq!(launch.children.len(), 1);
    assert!(launch.errors.is_empty());
    assert!(wait(launch.children[0].status()).unwrap().success());
    std::fs::write(
        user.join("fixture.desktop"),
        "[Desktop Entry]\nType=Application\nHidden=true\nExec=/bin/true\n",
    )
    .unwrap();
    assert!(
        wait(application("fixture.desktop").launch()).is_err(),
        "hidden user entry shadows system entry"
    );
    owner.close();
}

#[test]
#[cfg(target_os = "linux")]
fn journal_does_not_follow_symlink_or_replay_unknown_version() {
    let fixture = Fixture::new();
    let config = fixture.config();
    let env = Environment::inherited();
    let (journal, _) = recovery::Journal::open(&config, &env).unwrap().unwrap();
    let target = fixture.0.join("untouched");
    std::fs::write(&target, "safe").unwrap();
    std::os::unix::fs::symlink(
        &target,
        config
            .recovery_directory
            .as_ref()
            .unwrap()
            .join("session.tmp"),
    )
    .unwrap();
    assert!(journal.write(Vec::new()).is_err());
    assert_eq!(std::fs::read_to_string(target).unwrap(), "safe");
    drop(journal);
    std::fs::write(
        config.recovery_directory.unwrap().join("session.toml"),
        "version = 99\nentries = []\n",
    )
    .unwrap();
    assert!(recovery::Journal::open(&fixture.config(), &env).is_err());
}

#[test]
#[cfg(all(target_os = "linux", feature = "shell-xwayland"))]
fn x11_child_environment_withdrawal_suppresses_incident_retries() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let mut env = Environment::inherited();
    env.0.remove(std::ffi::OsStr::new("DISPLAY"));
    env.0.remove(std::ffi::OsStr::new("XAUTHORITY"));
    env.0
        .insert("WAYLAND_DISPLAY".into(), "wayland-fixture".into());
    let owner = SessionOwner::start(env, fixture.config()).unwrap();
    let handle = current().unwrap();
    assert_eq!(handle.environment("DISPLAY"), None);
    owner.set_x11_environment(Some((":123".into(), "/private/fixture-auth".into())));
    let output = wait(
        command("/bin/sh")
            .args([
                "-c",
                "printf '%s|%s|%s' \"$DISPLAY\" \"$XAUTHORITY\" \"$WAYLAND_DISPLAY\"",
            ])
            .output(),
    )
    .unwrap();
    assert_eq!(output.stdout, b":123|/private/fixture-auth|wayland-fixture");
    let log = fixture.0.join("attempts");
    let child = command("/bin/sh")
        .args([
            OsString::from("-c"),
            OsString::from("printf x >> \"$1\"; exit 1"),
            OsString::from("fixture"),
            log.as_os_str().into(),
        ])
        .restart(RestartPolicy::OnFailure)
        .recover(true)
        .spawn()
        .unwrap();
    wait_until(|| log.exists());
    owner.set_x11_environment(None);
    assert!(!wait(child.status()).unwrap().success());
    assert_eq!(std::fs::read(log).unwrap(), b"x");
    assert_eq!(handle.environment("DISPLAY"), None);
    assert_eq!(handle.environment("XAUTHORITY"), None);
    assert_eq!(
        handle.environment("WAYLAND_DISPLAY"),
        Some("wayland-fixture".into())
    );
    let output = wait(
        command("/bin/sh")
            .args([
                "-c",
                "printf '%s|%s' \"${DISPLAY-unset}\" \"$WAYLAND_DISPLAY\"",
            ])
            .output(),
    )
    .unwrap();
    assert_eq!(output.stdout, b"unset|wayland-fixture");
    assert_eq!(pending_recovery().unwrap().len(), 1);
    owner.close();
}

#[test]
#[cfg(all(target_os = "linux", feature = "shell-xwayland"))]
fn initial_launches_wait_for_x11_success_failure_or_session_close() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    for outcome in [0, 1, 2] {
        let fixture = Fixture::new();
        let mut env = Environment::inherited();
        env.0.remove(std::ffi::OsStr::new("DISPLAY"));
        let owner = SessionOwner::start_waiting_for_x11(env, fixture.config()).unwrap();
        let child = command("/bin/sh")
            .args(["-c", "printf '%s' \"${DISPLAY-native}\""])
            .stdout(Stream::Capture)
            .spawn()
            .unwrap();
        assert_eq!(child.id(), 0);
        assert!(child.try_status().unwrap().is_none());
        match outcome {
            0 => owner.set_x11_environment(Some((":123".into(), "/private/auth".into()))),
            1 => owner.set_x11_environment(None),
            _ => owner.close(),
        }
        if outcome == 2 {
            assert!(matches!(wait(child.output()), Err(Error::Closing)));
            assert_eq!(child.id(), 0);
        } else {
            let output = wait(child.output()).unwrap();
            assert_eq!(
                output.stdout,
                if outcome == 0 {
                    b":123".as_slice()
                } else {
                    b"native".as_slice()
                }
            );
            assert_ne!(child.id(), 0);
        }
        owner.close();
        drop(owner);
    }
}

#[test]
#[cfg(all(target_os = "linux", feature = "shell-xwayland"))]
fn deferred_recoverable_launch_survives_shutdown_without_a_process_identity() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner =
        SessionOwner::start_waiting_for_x11(Environment::inherited(), fixture.config()).unwrap();
    let child = command("/bin/false").recover(true).spawn().unwrap();
    assert_eq!(child.id(), 0);
    owner.close();
    assert!(matches!(wait(child.status()), Err(Error::Closing)));
    drop(owner);
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    let entries = pending_recovery().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(!entries[0].original_process_alive());
    let restored = entries[0].restore().unwrap();
    assert_ne!(restored.id(), 0);
    assert!(!wait(restored.status()).unwrap().success());
    next.close();
}

#[test]
#[cfg(target_os = "linux")]
fn pending_recovery_does_not_exhaust_process_slots() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let env = Environment::inherited();
    let (journal, _) = recovery::Journal::open(&fixture.config(), &env)
        .unwrap()
        .unwrap();
    // More recovery offers than the live-process limit must survive journal reload.
    let entries = (1..=257)
        .map(|id| recovery::RecoveryEntry {
            id,
            spec: command("/bin/true").recover(true).spec,
            pid: 0,
            process_identity: None,
            generation: id,
        })
        .collect();
    journal.write(entries).unwrap();
    drop(journal);
    let owner = SessionOwner::start(env, fixture.config()).unwrap();
    assert_eq!(pending_recovery().unwrap().len(), 257);
    let child = command("/bin/true").recover(true).spawn().unwrap();
    assert!(wait(child.status()).unwrap().success());
    assert_eq!(pending_recovery().unwrap().len(), 257);
    owner.close();
}

#[test]
#[cfg(all(target_os = "linux", feature = "shell-xwayland"))]
fn queued_launches_still_enforce_process_limit() {
    let _serial = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let owner =
        SessionOwner::start_waiting_for_x11(Environment::inherited(), fixture.config()).unwrap();
    for _ in 0..256 {
        command("/bin/true").spawn().unwrap();
    }
    assert!(matches!(
        command("/bin/true").spawn(),
        Err(Error::ProcessLimit)
    ));
    owner.close();
}

#[test]
#[cfg(target_os = "linux")]
fn registry_resolution_launch_and_old_session_handles() {
    let _guard = OWNER_TEST.lock().unwrap_or_else(|e| e.into_inner());
    let fixture = Fixture::new();
    let apps = fixture.0.join("applications");
    std::fs::create_dir(&apps).unwrap();
    std::fs::write(
        apps.join("editor.desktop"),
        "[Desktop Entry]\nType=Application\nExec=/bin/true\n",
    )
    .unwrap();
    let registry = ApplicationRegistry::new()
        .binary_directory("/bin")
        .desktop_entry_directory(&apps)
        .register(
            "echo",
            ApplicationSpec::executable("echo").arg("literal $HOME"),
        )
        .register("editor", ApplicationSpec::desktop_entry("editor.desktop"));
    let owner = SessionOwner::start(
        Environment::inherited(),
        fixture.config().applications(registry),
    )
    .unwrap();
    let applications = applications().unwrap();
    let helper = applications.executable("sh").unwrap();
    let mut command = std::process::Command::new("/bin/sh");
    command
        .args(["-c", "printf '%s' \"$TELORGON_HELPER_TEST\""])
        .env("TELORGON_HELPER_TEST", "private")
        .stdout(std::process::Stdio::piped());
    let output = helper
        .spawn_helper(&mut command)
        .unwrap()
        .wait_with_output()
        .unwrap();
    assert_eq!(output.stdout, b"private");
    let echo = applications.get("echo").unwrap();
    let output = wait(echo.command().unwrap().output()).unwrap();
    assert_eq!(output.stdout, b"literal $HOME\n");
    assert!(applications.get("unregistered").is_err());
    assert!(applications.executable("../bin/echo").is_err());
    assert!(
        applications
            .executable("no-such-telorgon-binary")
            .unwrap_err()
            .to_string()
            .contains("/bin")
    );
    let desktop = applications.get("editor").unwrap();
    assert!(desktop.executable_parts().is_err());
    let launched = wait(desktop.launch()).unwrap();
    assert_eq!(launched.children.len(), 1);
    assert!(
        applications
            .desktop_entry("missing.desktop")
            .unwrap_err()
            .to_string()
            .contains("XDG discovery disabled")
    );
    drop(owner);
    let next = SessionOwner::start(Environment::inherited(), fixture.config()).unwrap();
    assert!(matches!(echo.command(), Err(Error::Closing)));
    assert!(matches!(applications.get("echo"), Err(Error::Closing)));
    drop(next);
}

#[test]
fn registry_rejects_ambiguous_configuration() {
    assert!(
        ApplicationRegistry::new()
            .binary_directory("relative")
            .validate()
            .is_err()
    );
    assert!(
        ApplicationRegistry::new()
            .register("picker", ApplicationSpec::executable("one"))
            .register("picker", ApplicationSpec::executable("two"))
            .validate()
            .is_err()
    );
}

#[test]
#[cfg(target_os = "linux")]
fn desktop_settings_follow_managed_launches_and_session_lifetime() {
    use std::os::unix::fs::PermissionsExt;
    let _serial = OWNER_TEST.lock().unwrap();
    let fixture = Fixture::new();
    let mut env = Environment::inherited();
    env.0.insert("XDG_RUNTIME_DIR".into(), fixture.0.as_os_str().into());
    env.0.insert("XDG_CONFIG_DIRS".into(), "/existing/config:/etc/xdg".into());
    let original = env.clone();
    let config = fixture.config().desktop_settings(DesktopSettings::default());
    let owner = SessionOwner::start(env, config.clone()).unwrap();
    let handle = current().unwrap();
    let paths = handle.environment("XDG_CONFIG_DIRS").unwrap();
    let paths: Vec<_> = std::env::split_paths(&paths).collect();
    assert_eq!(&paths[1..], &[PathBuf::from("/existing/config"), PathBuf::from("/etc/xdg")]);
    let settings = paths[0].join("gtk-3.0/settings.ini");
    assert_eq!(std::fs::metadata(&paths[0]).unwrap().permissions().mode() & 0o777, 0o700);
    // Verify an actual managed child receives the generated config, without a launcher or
    // process-global environment mutation and without starting GTK/a desktop service.
    let output = wait(shell("IFS=:; set -- $XDG_CONFIG_DIRS; cat \"$1/gtk-3.0/settings.ini\"").output()).unwrap();
    assert!(output.status.success());
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "[Settings]\ngtk-decoration-layout=menu:minimize,maximize,close\n");
    assert_eq!(original.get("XDG_CONFIG_DIRS"), Some(std::ffi::OsStr::new("/existing/config:/etc/xdg")));
    owner.close();
    drop(owner);
    assert!(!settings.exists(), "session-owned defaults must be removed on shutdown");
    assert!(matches!(SessionOwner::start_gui(original, config), Err(Error::Unsupported(_))));
}

#[test]
#[cfg(target_os = "linux")]
fn desktop_settings_reject_injection_before_creating_runtime_files() {
    let fixture = Fixture::new();
    let env = Environment(BTreeMap::from([("XDG_RUNTIME_DIR".into(), fixture.0.as_os_str().into())]));
    let config = fixture.config().desktop_settings(DesktopSettings {
        decoration_layout: "menu:close\n[Settings]\ngtk-theme-name=bad".into(),
    });
    assert!(super::desktop_settings::prepare(env, &config).is_err());
    assert_eq!(std::fs::read_dir(&fixture.0).unwrap().count(), 0);
}

    #[test]
    fn recovery_journal_excludes_second_owner_and_can_be_reopened() {
        let fixture = Fixture::new();
        let config = fixture.config();
        let env = Environment(BTreeMap::new());
        let (journal, _) = recovery::Journal::open(&config, &env).unwrap().unwrap();
        assert!(matches!(recovery::Journal::open(&config, &env), Err(Error::RecoveryInUse)));
        journal.write(Vec::new()).unwrap();
        journal.write(Vec::new()).unwrap();
        drop(journal);
        assert!(recovery::Journal::open(&config, &env).unwrap().is_some());
    }

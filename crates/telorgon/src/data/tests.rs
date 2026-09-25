use super::*;
use crate::foundation::{ColorRgba8, SizeI};
use crate::ui::layout::ScrollPhysics;
use std::{
    path::PathBuf,
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Directory(PathBuf);
impl Directory {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "telorgon-data-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn file(&self) -> PathBuf {
        self.0.join("settings.toml")
    }
}
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn typed_handles_restore_foundation_values_and_preserve_unknown_keys() {
    let registry = Registry::new("test", 1);
    let color = registry
        .create("color", ColorRgba8::rgba(1, 2, 3, 4))
        .unwrap();
    let size = registry
        .create(
            "size",
            SizeI {
                width: 20,
                height: 30,
            },
        )
        .unwrap();
    let original = registry.to_toml().unwrap();
    color.set(ColorRgba8::rgba(5, 6, 7, 8)).unwrap();
    size.update(|s| s.width = 90).unwrap();
    registry.restore_toml(&original).unwrap();
    assert_eq!(color.clone().get().unwrap().a, 4);
    assert_eq!(size.get().unwrap().width, 20);
    assert!(matches!(
        registry.get::<u32>("color"),
        Err(DataError::TypeMismatch(_))
    ));
    assert!(matches!(
        registry.create("color", 1u32),
        Err(DataError::Duplicate(_))
    ));
    registry
        .restore_toml("registry='test'\nversion=1\n[values]\nfuture=42")
        .unwrap();
    assert_eq!(size.get().unwrap().width, 20);
    assert!(registry.to_toml().unwrap().contains("future = 42"));
    let future = registry.create("future", 0u32).unwrap();
    assert_eq!(future.get().unwrap(), 42);
    future.reset().unwrap();
    assert_eq!(future.get().unwrap(), 0);
}

#[test]
fn failed_restore_is_atomic_and_schema_is_checked() {
    let registry = Registry::new("test", 1);
    let a = registry.create("a", 1u32).unwrap();
    let b = registry
        .register(EntrySpec::new("b", 2u32).validate(|v| {
            if *v <= 10 {
                Ok(())
            } else {
                Err("must be at most ten".into())
            }
        }))
        .unwrap();
    let revision = registry.save_status().unwrap().revision;
    let invalid = "registry='test'\nversion=1\n[values]\na=9\nb=30";
    assert!(
        matches!(registry.restore_toml(invalid), Err(DataError::Value { key, .. }) if key == "b")
    );
    assert_eq!((a.get().unwrap(), b.get().unwrap()), (1, 2));
    assert_eq!(registry.save_status().unwrap().revision, revision);
    assert!(matches!(
        registry.restore_toml(&invalid.replace("version=1", "version=2")),
        Err(DataError::SchemaMismatch)
    ));
    assert!(b.update(|v| *v = 20).is_err());
    assert_eq!(b.get().unwrap(), 2);
}

#[test]
fn constrained_scroll_values_reject_invalid_saved_fields() {
    let registry = Registry::new("test", 1);
    let scroll = registry.create("scroll", ScrollPhysics::default()).unwrap();
    let original = scroll.get().unwrap();
    let bad = "registry='test'\nversion=1\n[values.scroll]\ndeceleration=-1.0\nstop_velocity=5.0";
    assert!(registry.restore_toml(bad).is_err());
    assert_eq!(scroll.get().unwrap(), original);
    registry.restore_toml(&registry.to_toml().unwrap()).unwrap();
}

#[test]
fn transactions_notify_as_one_batch_and_roll_back_on_error() {
    let registry = Registry::new("test", 1);
    let a = registry.create("a", 1u32).unwrap();
    let b = registry.create("b", 2u32).unwrap();
    let changes = registry.subscribe().unwrap();
    registry
        .transaction(|tx| {
            tx.set(&a, 10)?;
            tx.set(&b, 20)
        })
        .unwrap();
    let batch = changes.try_recv().unwrap().unwrap();
    assert_eq!(batch.keys.into_iter().collect::<Vec<_>>(), ["a", "b"]);
    let values = registry
        .read(|tx| Ok((tx.get::<u32>("a")?, tx.get::<u32>("b")?)))
        .unwrap();
    assert_eq!(values, (10, 20));
    let failed: DataResult<()> = registry.transaction(|tx| {
        tx.set(&a, 100)?;
        Err(DataError::Conflict)
    });
    assert!(failed.is_err());
    assert_eq!(a.get().unwrap(), 10);
    assert!(changes.try_recv().unwrap().is_none());
    for v in 0..100 {
        a.set(v).unwrap();
    }
    assert_eq!(changes.try_recv().unwrap().unwrap().keys.len(), 1);
    assert!(changes.try_recv().unwrap().is_none());
}

#[test]
fn reentrant_update_and_transaction_detect_conflicts_without_deadlocking() {
    let registry = Registry::new("test", 1);
    let value = registry.create("v", 1u32).unwrap();
    assert!(matches!(
        value.update(|v| {
            value.set(2).unwrap();
            *v = 3;
        }),
        Err(DataError::Conflict)
    ));
    assert_eq!(value.get().unwrap(), 2);
    assert!(matches!(
        registry.transaction(|tx| {
            tx.set(&value, 4)?;
            value.set(5)?;
            Ok(())
        }),
        Err(DataError::Conflict)
    ));
    assert_eq!(value.get().unwrap(), 5);
    drop(registry);
    value.set(6).unwrap();
    assert_eq!(value.get().unwrap(), 6);
}

#[test]
fn manual_persistence_and_io_failures_keep_pending_state() {
    let directory = Directory::new();
    let registry = Registry::new("test", 1);
    let value = registry.create("v", 1u32).unwrap();
    assert!(!registry.load_if_exists(directory.file()).unwrap());
    registry.save(directory.file()).unwrap();
    assert!(!registry.save_status().unwrap().dirty);
    value.set(2).unwrap();
    let missing = directory.0.join("missing/settings.toml");
    assert!(registry.save(&missing).is_err());
    assert!(registry.save_status().unwrap().dirty);
    assert!(registry.save_status().unwrap().last_error.is_some());
    registry.load(directory.file()).unwrap();
    assert_eq!(value.get().unwrap(), 1);
    assert!(!registry.save_status().unwrap().dirty);
}

#[derive(Clone, serde::Deserialize)]
struct PausingValue {
    number: u32,
    #[serde(skip)]
    gate: Option<Arc<Gate>>,
}
struct Gate {
    armed: AtomicBool,
    entered: Barrier,
    released: Barrier,
}
impl serde::Serialize for PausingValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        if let Some(gate) = &self.gate {
            if gate.armed.swap(false, Ordering::SeqCst) {
                gate.entered.wait();
                gate.released.wait();
            }
        }
        let mut output = serializer.serialize_struct("PausingValue", 1)?;
        output.serialize_field("number", &self.number)?;
        output.end()
    }
}

#[test]
fn changes_during_a_save_stay_dirty_and_snapshots_remain_consistent() {
    let directory = Directory::new();
    let registry = Arc::new(Registry::new("test", 1));
    let gate = Arc::new(Gate {
        armed: AtomicBool::new(false),
        entered: Barrier::new(2),
        released: Barrier::new(2),
    });
    let value = registry
        .create(
            "v",
            PausingValue {
                number: 1,
                gate: Some(gate.clone()),
            },
        )
        .unwrap();
    gate.armed.store(true, Ordering::SeqCst);
    let writer = registry.clone();
    let path = directory.file();
    let saving = std::thread::spawn(move || writer.save(path));
    gate.entered.wait();
    value
        .set(PausingValue {
            number: 2,
            gate: None,
        })
        .unwrap();
    gate.released.wait();
    saving.join().unwrap().unwrap();
    assert!(registry.save_status().unwrap().dirty);
    let file = std::fs::read_to_string(directory.file()).unwrap();
    assert!(file.contains("number = 1"));
    registry.save(directory.file()).unwrap();
    assert!(!registry.save_status().unwrap().dirty);
    assert!(
        std::fs::read_to_string(directory.file())
            .unwrap()
            .contains("number = 2")
    );
}

#[test]
fn autosave_saves_while_running_flushes_and_stops() {
    let directory = Directory::new();
    let registry = Registry::new("test", 1);
    let value = registry.create("v", 1u32).unwrap();
    registry
        .enable_autosave(
            Autosave::to(directory.file())
                .debounce(Duration::from_millis(20))
                .max_delay(Duration::from_millis(80)),
        )
        .unwrap();
    value.set(42).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while registry.save_status().unwrap().dirty {
        assert!(
            std::time::Instant::now() < deadline,
            "autosave did not complete"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(
        std::fs::read_to_string(directory.file())
            .unwrap()
            .contains("v = 42")
    );
    value.set(43).unwrap();
    registry.flush().unwrap();
    assert!(
        std::fs::read_to_string(directory.file())
            .unwrap()
            .contains("v = 43")
    );
    registry.shutdown().unwrap();
    value.set(44).unwrap();
    assert!(registry.save_status().unwrap().dirty);
    assert!(matches!(registry.flush(), Err(DataError::AutosaveDisabled)));
}

#[test]
fn autosave_failure_is_observable_and_retryable() {
    let directory = Directory::new();
    let path = directory.0.join("missing/settings.toml");
    let registry = Registry::new("test", 1);
    registry.create("v", true).unwrap();
    registry
        .enable_autosave(Autosave::to(&path).debounce(Duration::from_millis(10)))
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while registry.save_status().unwrap().last_error.is_none() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(registry.save_status().unwrap().dirty);
    std::fs::create_dir(path.parent().unwrap()).unwrap();
    registry.flush().unwrap();
    assert!(!registry.save_status().unwrap().dirty);
    assert!(registry.save_status().unwrap().last_error.is_none());
    registry.shutdown().unwrap();
}

#[test]
fn motion_round_trips_and_invalid_springs_are_rejected() {
    let registry = Registry::new("test", 1);
    let motion = registry
        .create("motion", crate::WindowMotion::fluid())
        .unwrap();
    let spring = registry.create("spring", crate::Spring::new()).unwrap();
    let text = registry.to_toml().unwrap();
    motion.set(crate::WindowMotion::none()).unwrap();
    registry.restore_toml(&text).unwrap();
    assert_eq!(motion.get().unwrap(), crate::WindowMotion::fluid());
    let invalid = text.replace("damping_ratio = 0.86", "damping_ratio = 1.2");
    assert!(registry.restore_toml(&invalid).is_err());
    assert_eq!(spring.get().unwrap(), crate::Spring::new());
}

#[test]
fn exporting_another_file_does_not_acknowledge_autosave() {
    let directory = Directory::new();
    let registry = Registry::new("test", 1);
    registry.create("v", 1u32).unwrap();
    registry
        .enable_autosave(
            Autosave::to(directory.file())
                .debounce(Duration::from_secs(60))
                .max_delay(Duration::from_secs(60)),
        )
        .unwrap();
    registry.save(directory.0.join("export.toml")).unwrap();
    assert!(registry.save_status().unwrap().dirty);
    assert!(!directory.file().exists());
    registry.shutdown().unwrap();
    assert!(directory.file().exists());
}

#[test]
fn continuous_edits_are_saved_by_the_maximum_delay() {
    let directory = Directory::new();
    let registry = Registry::new("test", 1);
    let value = registry.create("v", 0u32).unwrap();
    registry
        .enable_autosave(
            Autosave::to(directory.file())
                .debounce(Duration::from_secs(10))
                .max_delay(Duration::from_millis(30)),
        )
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut next = 1;
    while !directory.file().exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "continuous changes starved autosave"
        );
        value.set(next).unwrap();
        next += 1;
        std::thread::sleep(Duration::from_millis(2));
    }
    registry.shutdown().unwrap();
    assert!(!registry.save_status().unwrap().dirty);
}

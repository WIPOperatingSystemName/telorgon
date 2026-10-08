use super::artwork::{
    MAX_BOOT_ARTWORK_BYTES, MAX_PE_HEADER_BYTES, decode_artwork, embedded_splash_range,
};
use super::{
    BootApplication, BootError, BootResult, BootSelectionMode, BootSource, BootTarget,
    BootTargetKind, BootTheme,
};
use crate::ui::ImageId;
use serde::Deserialize;

pub const MAX_CONFIG_BYTES: usize = 64 * 1024;

/// Declarative configuration for the shared boot UI. Image paths are absolute paths
/// on the EFI volume containing the launcher, not desktop filesystem paths.
pub struct BootConfig {
    pub theme: BootTheme,
    pub selection_mode: BootSelectionMode,
    pub default_target: Option<String>,
    pub targets: Vec<BootTarget>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    #[serde(default = "default_theme")]
    theme: String,
    selection: Option<String>,
    default: Option<String>,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    id: String,
    name: String,
    #[serde(default = "unknown_kind")]
    kind: String,
    #[serde(default)]
    detail: String,
    image: Option<String>,
    arguments: Option<String>,
    icon: Option<String>,
    splash: Option<String>,
    #[serde(default = "enabled")]
    embedded_splash: bool,
}

fn unknown_kind() -> String {
    "unknown".into()
}
fn enabled() -> bool {
    true
}

fn default_theme() -> String {
    "disks".into()
}

impl BootConfig {
    pub fn from_toml(text: &str) -> BootResult<Self> {
        if text.len() > MAX_CONFIG_BYTES {
            return Err(BootError::Configuration(
                "boot configuration exceeds 64 KiB".into(),
            ));
        }
        let document: Document =
            toml::from_str(text).map_err(|error| BootError::Configuration(error.to_string()))?;
        if document.targets.is_empty() || document.targets.len() > 16 {
            return Err(BootError::Configuration(
                "choose between 1 and 16 targets".into(),
            ));
        }
        let theme = match document.theme.as_str() {
            "disks" => BootTheme::Disks,
            "voxel" => BootTheme::Voxel,
            _ => {
                return Err(BootError::Configuration(
                    "theme must be disks or voxel".into(),
                ));
            }
        };
        let selection_mode = match document.selection.as_deref().unwrap_or("auto") {
            "auto" => BootSelectionMode::Auto,
            "always" => BootSelectionMode::Always,
            _ => {
                return Err(BootError::Configuration(
                    "selection must be auto or always".into(),
                ));
            }
        };
        let mut targets = Vec::with_capacity(document.targets.len());
        for entry in document.targets {
            let kind = match entry.kind.as_str() {
                "linux" => BootTargetKind::Linux,
                "windows" => BootTargetKind::Windows,
                "custom" => BootTargetKind::Custom,
                _ => BootTargetKind::Unknown,
            };
            let mut target = BootTarget::new(&entry.id, &entry.name, kind)?.detail(&entry.detail);
            target.embedded_splash = entry.embedded_splash;
            if let Some(path) = entry.icon {
                target = target.icon_file(&path)?;
            }
            if let Some(path) = entry.splash {
                target = target.splash_file(&path)?;
            }
            if let Some(image) = entry.image {
                let mut source = BootSource::efi(&image)?;
                if let Some(arguments) = entry.arguments {
                    source = source.arguments(&arguments)?;
                }
                target = target.source(source);
            } else if entry.arguments.is_some() {
                return Err(BootError::Configuration(
                    "arguments require an EFI image".into(),
                ));
            }
            targets.push(target);
        }
        let config = Self {
            theme,
            selection_mode,
            default_target: document.default,
            targets,
        };
        config.application()?.build()?;
        Ok(config)
    }

    /// A normal builder so callers can supply their own BootScreens factory.
    pub fn application(&self) -> BootResult<BootApplication> {
        let mut app = BootApplication::new()
            .theme(self.theme)
            .selection_mode(self.selection_mode);
        for target in &self.targets {
            app = app.target(target.clone());
        }
        if let Some(default) = &self.default_target {
            app = app.default_target(default);
        }
        Ok(app)
    }

    /// Resolves optional artwork before UI construction. Failed artwork never
    /// prevents a target from booting; the view retains its OS-specific fallback.
    pub fn load_artwork<E: core::fmt::Display>(
        &mut self,
        mut read: impl FnMut(&str, u64, usize) -> Result<Vec<u8>, E>,
    ) -> Vec<String> {
        let mut warnings = Vec::new();
        for (index, target) in self.targets.iter_mut().enumerate() {
            let base = 0xb008_0000 + index as u32 * 4;
            for (path, resource, id) in [
                (&target.icon_path, &mut target.icon, ImageId(base)),
                (&target.splash_path, &mut target.splash, ImageId(base + 1)),
            ] {
                let Some(path) = path else { continue };
                let decoded = read(path, 0, MAX_BOOT_ARTWORK_BYTES + 1)
                    .map_err(|error| error.to_string())
                    .and_then(|bytes| {
                        decode_artwork(&bytes, id).map_err(|error| error.to_string())
                    });
                match decoded {
                    Ok(image) => *resource = Some(image),
                    Err(error) => {
                        warnings.push(format!("{}: artwork {path}: {error}", target.name))
                    }
                }
            }
            if target.splash.is_some() || !target.embedded_splash {
                continue;
            }
            let Some(source) = &target.source else {
                continue;
            };
            let decoded = (|| -> Result<_, String> {
                let header = read(source.path(), 0, MAX_PE_HEADER_BYTES)
                    .map_err(|error| error.to_string())?;
                let Some(range) =
                    embedded_splash_range(&header).map_err(|error| error.to_string())?
                else {
                    return Ok(None);
                };
                let bytes = read(source.path(), range.start as u64, range.len())
                    .map_err(|error| error.to_string())?;
                if bytes.len() != range.len() {
                    return Err("truncated embedded splash".into());
                }
                decode_artwork(&bytes, ImageId(base + 2))
                    .map(Some)
                    .map_err(|error| error.to_string())
            })();
            match decoded {
                Ok(image) => target.splash = image,
                Err(error) => warnings.push(format!("{}: embedded splash: {error}", target.name)),
            }
        }
        warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unrecognized_or_missing_os_uses_unknown_kind() {
        for kind in ["", "kind='unrecognized-os'\n", "kind='unknown'\n"] {
            let config = BootConfig::from_toml(&format!(
                "[[targets]]\nid='os'\nname='Operating system'\n{kind}image='\\EFI\\OS\\boot.efi'"
            ))
            .unwrap();
            assert_eq!(config.targets[0].kind, BootTargetKind::Unknown);
            assert!(config.targets[0].source.is_some());
        }
    }

    #[test]
    fn missing_or_corrupt_artwork_keeps_target_bootable() {
        let mut config = BootConfig::from_toml(
            "[[targets]]\nid='os'\nname='Operating system'\nimage='\\EFI\\OS\\boot.efi'\nicon='\\EFI\\OS\\missing.png'\nsplash='\\EFI\\OS\\corrupt.bmp'"
        ).unwrap();
        let warnings = config.load_artwork(|path, _, _| -> Result<Vec<u8>, &'static str> {
            if path.ends_with("missing.png") {
                Err("not found")
            } else {
                Ok(vec![0; 32])
            }
        });
        assert_eq!(warnings.len(), 3);
        assert!(config.targets[0].icon.is_none());
        assert!(config.targets[0].splash.is_none());
        assert!(
            config
                .application()
                .unwrap()
                .build()
                .unwrap()
                .into_session()
                .is_ok()
        );
    }

    #[test]
    fn embedded_lookup_reads_headers_and_payload_without_scanning_kernel() {
        use image::ImageEncoder;
        let mut bmp = Vec::new();
        image::codecs::bmp::BmpEncoder::new(&mut bmp)
            .write_image(&[24, 72, 120], 1, 1, image::ExtendedColorType::Rgb8)
            .unwrap();
        let offset = 1024 * 1024;
        let mut header = vec![0u8; 512];
        header[..2].copy_from_slice(b"MZ");
        header[0x3c..0x40].copy_from_slice(&64u32.to_le_bytes());
        header[64..68].copy_from_slice(b"PE\0\0");
        header[70..72].copy_from_slice(&1u16.to_le_bytes());
        header[84..86].copy_from_slice(&240u16.to_le_bytes());
        header[88..90].copy_from_slice(&0x20bu16.to_le_bytes());
        let section = 64 + 24 + 240;
        header[section..section + 8].copy_from_slice(b".splash\0");
        header[section + 8..section + 12].copy_from_slice(&(bmp.len() as u32).to_le_bytes());
        header[section + 16..section + 20].copy_from_slice(&(bmp.len() as u32).to_le_bytes());
        header[section + 20..section + 24].copy_from_slice(&(offset as u32).to_le_bytes());
        let mut config = BootConfig::from_toml(
            "[[targets]]\nid='os'\nname='Operating system'\nimage='\\EFI\\OS\\boot.efi'",
        )
        .unwrap();
        let mut reads = Vec::new();
        let warnings = config.load_artwork(|_, start, length| -> Result<Vec<u8>, &'static str> {
            reads.push((start, length));
            if start == 0 {
                Ok(header.clone())
            } else {
                Ok(bmp.clone())
            }
        });
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(
            reads,
            [(0, MAX_PE_HEADER_BYTES), (offset as u64, bmp.len())]
        );
        let splash = config.targets[0].splash.as_ref().unwrap();
        assert_eq!(splash.pixels.as_ref(), &[24, 72, 120, 255]);
    }

    #[test]
    fn artwork_paths_reject_parent_traversal() {
        assert!(
            BootConfig::from_toml("[[targets]]\nid='os'\nname='OS'\nicon='\\EFI\\..\\icon.png'")
                .is_err()
        );
    }

    #[test]
    fn configuration_preserves_sources_options_and_shared_theme() {
        let config = BootConfig::from_toml(
            "theme='voxel'\ndefault='linux'\n[[targets]]\nid='linux'\nname='Linux'\nkind='linux'\nimage='\\EFI\\Linux\\linux.efi'\narguments='quiet root=UUID=example'\n",
        ).unwrap();
        assert_eq!(config.theme, BootTheme::Voxel);
        assert_eq!(config.selection_mode, BootSelectionMode::Auto);
        let session = config
            .application()
            .unwrap()
            .build()
            .unwrap()
            .into_session()
            .unwrap();
        assert_eq!(
            session.controller.snapshot().phase,
            super::super::BootPhase::Loading
        );
        assert!(!session.controller.snapshot().simulation);
        let BootSource::EfiImage { path, options, .. } = session.controller.snapshot().targets[0]
            .source
            .clone()
            .unwrap();
        assert_eq!(path, "\\EFI\\Linux\\linux.efi");
        assert_eq!(options.last(), Some(&0));
    }

    #[test]
    fn selection_policy_is_explicit_and_defaults_to_automatic_boot() {
        let target = "\n[[targets]]\nid='linux'\nname='Linux'\nkind='linux'\nimage='\\EFI\\Linux\\linux.efi'\n";
        for prefix in ["", "selection='auto'"] {
            let config = BootConfig::from_toml(&format!("{prefix}{target}")).unwrap();
            assert!(config.application().unwrap().build().unwrap().auto_boots());
        }
        let config = BootConfig::from_toml(&format!("selection='always'{target}")).unwrap();
        assert_eq!(config.selection_mode, BootSelectionMode::Always);
        let session = config
            .application()
            .unwrap()
            .build()
            .unwrap()
            .into_session()
            .unwrap();
        assert_eq!(
            session.controller.snapshot().phase,
            super::super::BootPhase::Selecting
        );
        assert!(session.controller.take_request().is_none());
        assert!(BootConfig::from_toml(&format!("selection='sometimes'{target}")).is_err());
    }

    #[test]
    fn bad_configuration_cannot_silently_change_boot_targets() {
        for text in [
            "theme='unknown'\ntargets=[]",
            "targets=[]\nmisspelled=true",
            "[[targets]]\nid='linux'\nname='Linux'\nkind='linux'\nimage='../kernel.efi'",
            "default='missing'\n[[targets]]\nid='linux'\nname='Linux'\nkind='linux'",
        ] {
            assert!(BootConfig::from_toml(text).is_err());
        }
        assert!(BootConfig::from_toml(&"x".repeat(MAX_CONFIG_BYTES + 1)).is_err());
    }
}

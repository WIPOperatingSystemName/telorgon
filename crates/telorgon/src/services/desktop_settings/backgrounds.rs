use super::{
    store::{private_directory, read_regular},
    *,
};
use crate::{
    ImageId, SizeI,
    graphics::render::{ImageAlphaMode, ImageColorEncoding, ImagePixelFormat, ImageResource},
};
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Cursor,
    path::{Path, PathBuf},
};

const MAX_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_DECODED_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundEntry {
    pub settings: PersonalizationSettings,
    pub label: String,
    pub error: Option<String>,
    // Even failed tiles change when bytes change, rather than caching a stale failure forever.
    fingerprint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedBackground {
    settings: PersonalizationSettings,
    directory: PathBuf,
    digest: String,
}
impl ValidatedBackground {
    pub fn settings(&self) -> &PersonalizationSettings {
        &self.settings
    }
    pub fn verify(&self, store: &SettingsStore) -> Result<()> {
        let _lock = store.lock()?;
        self.verify_locked(store)
    }
    pub(crate) fn verify_locked(&self, store: &SettingsStore) -> Result<()> {
        let (_, validated) = store.read_background(&self.settings)?;
        if validated != *self {
            return Err("The validated background changed or belongs to another library".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreparedBackgroundThumbnail {
    pub image: ImageResource,
    pub validated: ValidatedBackground,
}

impl SettingsStore {
    pub fn background_directory(&self) -> Result<PathBuf> {
        Ok(self.parent()?.join("backgrounds"))
    }
    pub fn ensure_background_directory(&self) -> Result<()> {
        let _lock = self.lock()?;
        self.ensure_library()
    }
    fn ensure_library(&self) -> Result<()> {
        private_directory(&self.background_directory()?)
    }
    pub fn background_path(&self, settings: &PersonalizationSettings) -> Result<Option<PathBuf>> {
        settings.validate()?;
        Ok(settings
            .background
            .as_ref()
            .map(|name| self.background_directory().map(|dir| dir.join(name)))
            .transpose()?)
    }
    pub(crate) fn read_background(
        &self,
        settings: &PersonalizationSettings,
    ) -> Result<(Vec<u8>, ValidatedBackground)> {
        let path = self
            .background_path(settings)?
            .ok_or("The default background has no library file")?;
        let directory = self.background_directory()?;
        if !fs::symlink_metadata(&directory)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("Background library must be a real directory".into());
        }
        let bytes = read_regular(&path, MAX_IMAGE_BYTES)
            .map_err(|e| format!("Cannot read background: {e}"))?;
        let digest = digest(&bytes);
        if settings.background.as_deref() != Some(&format!("background-{digest}.png")) {
            return Err("Background content does not match its managed filename".into());
        }
        Ok((
            bytes,
            ValidatedBackground {
                settings: settings.clone(),
                directory: fs::canonicalize(directory).map_err(|e| e.to_string())?,
                digest,
            },
        ))
    }
    pub fn import_background(&self, source: &Path) -> Result<PersonalizationSettings> {
        Ok(self.import(source, None)?.0.settings)
    }
    pub fn import_background_with_thumbnail(
        &self,
        source: &Path,
        image: ImageId,
    ) -> Result<PreparedBackgroundThumbnail> {
        let (validated, thumbnail) = self.import(source, Some(image))?;
        Ok(PreparedBackgroundThumbnail {
            validated,
            image: thumbnail.expect("requested thumbnail"),
        })
    }
    fn import(
        &self,
        source: &Path,
        thumbnail: Option<ImageId>,
    ) -> Result<(ValidatedBackground, Option<ImageResource>)> {
        // Decode once outside the library lock. Never retain a reference to the original user file.
        let bytes = read_regular(source, MAX_IMAGE_BYTES)
            .map_err(|e| format!("Cannot import image: {e}"))?;
        let decoded = decode_image(&bytes)?;
        let mut encoded = Cursor::new(Vec::new());
        decoded
            .write_to(&mut encoded, ImageFormat::Png)
            .map_err(|e| e.to_string())?;
        let encoded = encoded.into_inner();
        if encoded.len() as u64 > MAX_IMAGE_BYTES {
            return Err("Encoded wallpaper is too large".into());
        }
        let digest = digest(&encoded);
        let settings = PersonalizationSettings {
            background: Some(format!("background-{digest}.png")),
        };
        let preview = thumbnail.map(|id| resource(decoded.thumbnail(320, 180), id));
        let _lock = self.lock()?;
        self.ensure_library()?;
        let path = self.background_path(&settings)?.unwrap();
        if let Ok(metadata) = fs::symlink_metadata(&path) {
            if !metadata.is_file() {
                return Err("Managed wallpaper must be a regular file".into());
            }
        }
        crate::data::replace_file(&path, &encoded).map_err(|e| e.to_string())?;
        let validated = ValidatedBackground {
            settings,
            directory: fs::canonicalize(self.background_directory()?).map_err(|e| e.to_string())?,
            digest,
        };
        Ok((validated, preview))
    }
    pub fn list_backgrounds(&self) -> Result<Vec<BackgroundEntry>> {
        let _lock = self.lock()?;
        self.ensure_library()?;
        let mut entries = Vec::new();
        for entry in fs::read_dir(self.background_directory()?).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            let settings = PersonalizationSettings {
                background: Some(name.clone()),
            };
            if settings.validate().is_err() {
                continue;
            }
            if entries.len() >= 4096 {
                return Err("Background library contains too many images".into());
            }
            let bytes = read_regular(&entry.path(), MAX_IMAGE_BYTES);
            let fingerprint = bytes.as_ref().ok().map(|bytes| digest(bytes));
            let error = match bytes {
                Err(error) => Some(error.to_string()),
                Ok(_)
                    if fingerprint
                        .as_ref()
                        .is_none_or(|d| name != format!("background-{d}.png")) =>
                {
                    Some("Background content does not match its managed filename".into())
                }
                Ok(_) => None,
            };
            entries.push(BackgroundEntry {
                settings,
                label: format!("Wallpaper {}", &name[11..23]),
                error,
                fingerprint,
            });
        }
        entries.sort_by(|a, b| a.settings.background.cmp(&b.settings.background));
        Ok(entries)
    }
    pub fn load_background_validated(
        &self,
        settings: &PersonalizationSettings,
        image: ImageId,
    ) -> Result<(Option<ImageResource>, Option<ValidatedBackground>)> {
        settings.validate()?;
        if settings.background.is_none() {
            return Ok((None, None));
        }
        let (bytes, validated) = {
            let _lock = self.lock()?;
            self.read_background(settings)?
        };
        Ok((
            Some(resource(decode_image(&bytes)?, image)),
            Some(validated),
        ))
    }
    pub fn load_background_thumbnail_validated(
        &self,
        settings: &PersonalizationSettings,
        image: ImageId,
    ) -> Result<PreparedBackgroundThumbnail> {
        let (bytes, validated) = {
            let _lock = self.lock()?;
            self.read_background(settings)?
        };
        Ok(PreparedBackgroundThumbnail {
            image: resource(decode_image(&bytes)?.thumbnail(320, 180), image),
            validated,
        })
    }
    pub fn delete_background(&self, settings: &PersonalizationSettings) -> Result<()> {
        settings.validate()?;
        let path = self
            .background_path(settings)?
            .ok_or("The default background cannot be deleted")?;
        let _lock = self.lock()?;
        if self.load_personalization()? == *settings {
            return Err(
                "Choose and save another background before deleting the saved background".into(),
            );
        }
        let directory = self.background_directory()?;
        if !fs::symlink_metadata(&directory)
            .map_err(|e| e.to_string())?
            .is_dir()
        {
            return Err("Background library must be a real directory".into());
        }
        if !fs::symlink_metadata(&path)
            .map_err(|e| e.to_string())?
            .is_file()
        {
            return Err("Managed wallpaper must be a regular file".into());
        }
        fs::remove_file(path).map_err(|e| e.to_string())?;
        fs::File::open(directory)
            .and_then(|f| f.sync_all())
            .map_err(|e| e.to_string())
    }
}

fn digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub(super) fn decode_image(bytes: &[u8]) -> Result<DynamicImage> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| e.to_string())?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16384);
    limits.max_image_height = Some(16384);
    limits.max_alloc = Some(MAX_DECODED_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|e| e.to_string())?;
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 || u64::from(width) * u64::from(height) * 4 > MAX_DECODED_BYTES {
        return Err("Wallpaper dimensions exceed the decoded image limit".into());
    }
    let orientation = decoder.orientation().map_err(|e| e.to_string())?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|e| e.to_string())?;
    image.apply_orientation(orientation);
    Ok(image)
}
fn resource(image: DynamicImage, id: ImageId) -> ImageResource {
    let rgba = image.to_rgba8();
    ImageResource {
        image: id,
        content_version: 1,
        extent: SizeI {
            width: rgba.width() as i32,
            height: rgba.height() as i32,
        },
        color_encoding: ImageColorEncoding::Srgb,
        alpha_mode: ImageAlphaMode::Straight,
        pixel_format: ImagePixelFormat::Rgba8,
        pixels: rgba.into_raw().into(),
    }
}

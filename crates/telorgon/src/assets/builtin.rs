use super::{AssetBundle, AssetCatalogError, AssetEntry, AssetKey};

/// SDK-owned resources included by the enabled Cargo features.
pub const fn bundle() -> AssetBundle {
    AssetBundle::new(super::fonts::ENTRIES)
}

/// Combines project and SDK catalogs without copying their entries.
#[derive(Clone, Copy, Debug)]
pub struct AssetResolver {
    project: AssetBundle,
}

impl AssetResolver {
    pub fn new(project: AssetBundle) -> Result<Self, AssetCatalogError> {
        project.validate()?;
        if std::ptr::eq(project.iter().as_slice(), bundle().iter().as_slice()) {
            return Ok(Self {
                project: AssetBundle::EMPTY,
            });
        }
        for entry in project.iter() {
            if entry.key.as_str().starts_with("telorgon/") {
                return Err(AssetCatalogError::ReservedKey(entry.key));
            }
        }
        Ok(Self { project })
    }

    pub fn get(self, key: AssetKey) -> Option<&'static AssetEntry> {
        if key.as_str().starts_with("telorgon/") {
            bundle().get(key)
        } else {
            self.project.get(key)
        }
    }

    pub fn iter(self) -> impl Iterator<Item = &'static AssetEntry> {
        self.project.iter().chain(bundle().iter())
    }
}

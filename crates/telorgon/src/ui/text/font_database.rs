use super::{TextResult, shaping::embedded_faces};
use cosmic_text::{FontSystem, fontdb::Database};
use std::sync::OnceLock;

type Seed = (String, Database);

pub(super) fn seed(system_fonts: bool) -> TextResult<&'static Seed> {
    static SYSTEM: OnceLock<TextResult<Seed>> = OnceLock::new();
    static BUNDLED: OnceLock<TextResult<Seed>> = OnceLock::new();
    let cell = if system_fonts { &SYSTEM } else { &BUNDLED };
    cell.get_or_init(|| {
        let (locale, mut db) = if system_fonts {
            let fonts = FontSystem::new();
            (fonts.locale().to_owned(), fonts.db().clone())
        } else {
            ("en-US".to_owned(), Database::new())
        };
        let mut faces = Vec::new();
        for entry in crate::assets::builtin::bundle().iter() {
            if entry.kind == crate::AssetKind::Font {
                faces.extend(embedded_faces(entry.bytes)?.iter().cloned());
            }
        }
        // System-installed versions must not win over the pinned SDK font files.
        let duplicates: Vec<_> = db
            .faces()
            .filter(|face| {
                face.families.iter().any(|(name, _)| {
                    faces.iter().any(|bundled| {
                        bundled
                            .families
                            .iter()
                            .any(|(candidate, _)| candidate.eq_ignore_ascii_case(name))
                    })
                })
            })
            .map(|face| face.id)
            .collect();
        for id in duplicates {
            db.remove_face(id);
        }
        for face in faces {
            db.push_face_info(face);
        }
        Ok((locale, db))
    })
    .as_ref()
    .map_err(Clone::clone)
}

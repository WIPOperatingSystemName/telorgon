//! Validated ICCCM size metadata. Policy decides which requested constraints to honor.
use crate::foundation::SizeI;
use x11rb_protocol::{protocol::xproto, x11_utils::TryParse};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AspectRatio {
    pub numerator: i32,
    pub denominator: i32,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NormalHints {
    pub user_position: bool,
    pub user_size: bool,
    pub program_position: bool,
    pub program_size: bool,
    pub minimum: Option<SizeI>,
    pub maximum: Option<SizeI>,
    pub increment: Option<SizeI>,
    pub aspect: Option<(AspectRatio, AspectRatio)>,
    /// Keep explicit base distinct: minimum substitutes for increments, not aspect ratios.
    pub base: Option<SizeI>,
    /// X gravity value 1..=10; absent defaults to NorthWest (1).
    pub gravity: Option<u32>,
}
impl NormalHints {
    /// Test client dimensions without rounding or searching on the owner thread.
    /// Uses exact integer aspect comparisons and explicit base subtraction only.
    pub fn accepts_size(self, size: SizeI) -> bool {
        if size.width <= 0 || size.height <= 0 {
            return false;
        }
        let min = self.minimum_size();
        if min.width < 0 || min.height < 0 || size.width < min.width || size.height < min.height {
            return false;
        }
        if self
            .maximum
            .is_some_and(|max| size.width > max.width || size.height > max.height)
        {
            return false;
        }
        if let Some(increment) = self.increment {
            let base = self.increment_base();
            if base.width < 0 || base.height < 0 || increment.width <= 0 || increment.height <= 0 {
                return false;
            }
            let width = i64::from(size.width) - i64::from(base.width);
            let height = i64::from(size.height) - i64::from(base.height);
            if width < 0
                || height < 0
                || width % i64::from(increment.width) != 0
                || height % i64::from(increment.height) != 0
            {
                return false;
            }
        }
        if let Some((min, max)) = self.aspect {
            let base = self.base.unwrap_or_default();
            if base.width < 0
                || base.height < 0
                || [
                    min.numerator,
                    min.denominator,
                    max.numerator,
                    max.denominator,
                ]
                .into_iter()
                .any(|v| v <= 0)
            {
                return false;
            }
            let width = i64::from(size.width) - i64::from(base.width);
            let height = i64::from(size.height) - i64::from(base.height);
            if width <= 0
                || height <= 0
                || width * i64::from(min.denominator) < height * i64::from(min.numerator)
                || width * i64::from(max.denominator) > height * i64::from(max.numerator)
            {
                return false;
            }
        }
        true
    }
    pub fn minimum_size(self) -> SizeI {
        self.minimum.or(self.base).unwrap_or_default()
    }
    pub fn increment_base(self) -> SizeI {
        self.base.or(self.minimum).unwrap_or_default()
    }
}

pub(crate) fn parse(bytes: &[u8]) -> Option<NormalHints> {
    if bytes.len() > 104 {
        return None;
    }
    let (reply, _) = xproto::GetPropertyReply::try_parse(bytes).ok()?;
    if reply.type_ == 0 && reply.format == 0 && reply.value_len == 0 && reply.bytes_after == 0 {
        return Some(NormalHints::default());
    }
    if reply.type_ != u32::from(xproto::AtomEnum::WM_SIZE_HINTS)
        || reply.format != 32
        || reply.bytes_after != 0
        || ![15, 18].contains(&reply.value_len)
    {
        return None;
    }
    let words: Vec<i32> = reply
        .value
        .chunks_exact(4)
        .map(|word| i32::from_ne_bytes(word.try_into().unwrap()))
        .collect();
    let flags = words[0] as u32;
    let has = |bit| flags & bit != 0;
    let size = |index: usize| SizeI {
        width: words[index],
        height: words[index + 1],
    };
    let mut hints = NormalHints {
        user_position: has(1),
        user_size: has(2),
        program_position: has(4),
        program_size: has(8),
        minimum: has(16).then(|| size(5)),
        maximum: has(32).then(|| size(7)),
        increment: has(64).then(|| size(9)),
        ..Default::default()
    };
    // Pre-ICCCM records do not supply base/gravity, even if stray flag bits are set.
    if words.len() == 18 {
        hints.base = has(256).then(|| size(15));
        hints.gravity = has(512).then(|| words[17] as u32);
    }
    for extent in [hints.minimum, hints.base].into_iter().flatten() {
        if extent.width < 0 || extent.height < 0 {
            return None;
        }
    }
    for extent in [hints.maximum, hints.increment].into_iter().flatten() {
        if extent.width <= 0 || extent.height <= 0 {
            return None;
        }
    }
    if let Some(max) = hints.maximum {
        let min = hints.minimum_size();
        if min.width > max.width || min.height > max.height {
            return None;
        }
    }
    if hints
        .gravity
        .is_some_and(|gravity| !(1..=10).contains(&gravity))
    {
        return None;
    }
    if has(128) {
        let min = AspectRatio {
            numerator: words[11],
            denominator: words[12],
        };
        let max = AspectRatio {
            numerator: words[13],
            denominator: words[14],
        };
        if [
            min.numerator,
            min.denominator,
            max.numerator,
            max.denominator,
        ]
        .into_iter()
        .any(|v| v <= 0)
            || i64::from(min.numerator) * i64::from(max.denominator)
                > i64::from(max.numerator) * i64::from(min.denominator)
        {
            return None;
        }
        hints.aspect = Some((min, max));
    }
    Some(hints)
}

#[cfg(test)]
mod tests {
    use super::*;
    use x11rb_protocol::x11_utils::Serialize;
    fn reply(words: &[i32]) -> Vec<u8> {
        xproto::GetPropertyReply {
            format: 32,
            type_: xproto::AtomEnum::WM_SIZE_HINTS.into(),
            value_len: words.len() as u32,
            length: words.len() as u32,
            value: words.iter().flat_map(|word| word.to_ne_bytes()).collect(),
            ..Default::default()
        }
        .serialize()
    }
    fn record() -> [i32; 18] {
        [
            1023, 999, 999, 999, 999, 80, 60, 800, 600, 8, 16, 1, 2, 2, 1, 0, 0, 1,
        ]
    }
    #[test]
    fn size_constraints_use_exact_increments_and_explicit_aspect_base() {
        let square = AspectRatio {
            numerator: 1,
            denominator: 1,
        };
        let mut hints = NormalHints {
            minimum: Some(SizeI {
                width: 80,
                height: 60,
            }),
            increment: Some(SizeI {
                width: 10,
                height: 10,
            }),
            aspect: Some((square, square)),
            ..Default::default()
        };
        assert!(hints.accepts_size(SizeI {
            width: 100,
            height: 100
        }));
        assert!(!hints.accepts_size(SizeI {
            width: 100,
            height: 80
        }));
        hints.base = hints.minimum;
        assert!(hints.accepts_size(SizeI {
            width: 100,
            height: 80
        }));
        assert!(!hints.accepts_size(SizeI {
            width: 101,
            height: 81
        }));
        assert!(!hints.accepts_size(SizeI {
            width: 80,
            height: 60
        }));
        hints.maximum = Some(SizeI {
            width: 100,
            height: 80,
        });
        assert!(!hints.accepts_size(SizeI {
            width: 110,
            height: 90
        }));
        hints.increment = Some(SizeI {
            width: 0,
            height: 1,
        });
        assert!(!hints.accepts_size(SizeI {
            width: 100,
            height: 80
        }));
        let large = AspectRatio {
            numerator: i32::MAX,
            denominator: i32::MAX,
        };
        assert!(
            NormalHints {
                aspect: Some((large, large)),
                ..Default::default()
            }
            .accepts_size(SizeI {
                width: i32::MAX,
                height: i32::MAX
            })
        );
        assert!(!NormalHints::default().accepts_size(SizeI::default()));
    }
    #[test]
    fn reads_modern_and_legacy_without_treating_padding_as_geometry() {
        let words = record();
        let hints = parse(&reply(&words)).unwrap();
        assert!(
            hints.user_position && hints.user_size && hints.program_position && hints.program_size
        );
        assert_eq!(
            hints.minimum_size(),
            SizeI {
                width: 80,
                height: 60
            }
        );
        assert_eq!(hints.increment_base(), SizeI::default());
        assert_eq!(
            hints.increment,
            Some(SizeI {
                width: 8,
                height: 16
            })
        );
        assert_eq!(hints.gravity, Some(1));
        let legacy = parse(&reply(&words[..15])).unwrap();
        assert_eq!(legacy.base, None);
        assert_eq!(legacy.gravity, None);
        assert_eq!(legacy.increment_base(), legacy.minimum_size());
        assert_eq!(legacy.aspect, hints.aspect);
    }
    #[test]
    fn validates_flagged_fields_and_ignores_unused_values() {
        for (index, value) in [
            (5, -1),
            (7, 1),
            (9, 0),
            (12, 0),
            (13, 0),
            (15, -1),
            (17, 11),
        ] {
            let mut words = record();
            words[index] = value;
            assert_eq!(parse(&reply(&words)), None, "field {index}");
        }
        let mut words = record();
        words[11] = 3;
        words[12] = 1;
        assert_eq!(parse(&reply(&words)), None);
        words.fill(-1);
        words[0] = 0;
        assert_eq!(parse(&reply(&words)), Some(NormalHints::default()));
        let absent = xproto::GetPropertyReply::default().serialize();
        assert_eq!(parse(&absent), Some(NormalHints::default()));
        let valid = reply(&record());
        for length in 0..valid.len() {
            assert_eq!(parse(&valid[..length]), None);
        }
        for length in [14, 16, 17, 19] {
            assert_eq!(parse(&reply(&vec![0; length])), None);
        }
        let mut wrong_type = valid.clone();
        wrong_type[8..12].copy_from_slice(&4u32.to_ne_bytes());
        assert_eq!(parse(&wrong_type), None);
        let mut remainder = valid;
        remainder[12..16].copy_from_slice(&4u32.to_ne_bytes());
        assert_eq!(parse(&remainder), None);
    }
}

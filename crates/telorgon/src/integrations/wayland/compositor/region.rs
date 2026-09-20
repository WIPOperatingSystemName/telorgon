use std::fmt;
use std::sync::Arc;

use crate::foundation::RectI;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Region {
    rectangles: Arc<[RectI]>,
}

impl Region {
    pub const MAX_RECTANGLES: usize = 256;

    pub fn empty() -> Self {
        Self::default()
    }

    pub fn from_rectangles(rectangles: Vec<RectI>) -> Result<Self, RegionError> {
        if rectangles.len() > Self::MAX_RECTANGLES {
            return Err(RegionError::TooManyRectangles);
        }
        if rectangles
            .iter()
            .any(|rect| rect.width <= 0 || rect.height <= 0)
        {
            return Err(RegionError::InvalidRectangle);
        }
        Ok(Self {
            rectangles: rectangles.into(),
        })
    }

    pub fn rectangles(&self) -> &[RectI] {
        &self.rectangles
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionError {
    TooManyRectangles,
    InvalidRectangle,
}

impl fmt::Display for RegionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyRectangles => "Wayland region exceeds its bounded rectangle capacity",
            Self::InvalidRectangle => "Wayland region contains a non-positive rectangle",
        })
    }
}

impl std::error::Error for RegionError {}

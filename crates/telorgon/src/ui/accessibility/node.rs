use std::fmt;
use std::sync::Arc;

use crate::foundation::{RectF, Transform2D};
use crate::ui::{SemanticNode, SemanticParticipation, StringId};

use crate::ui::accessibility::{SemanticNodeId, SemanticTreeError};

/// Hard bound for one resolved semantic string.
pub const MAX_SEMANTIC_STRING_BYTES: usize = 64 * 1024;
/// Hard bound for direct children exported by one semantic node.
pub const MAX_SEMANTIC_CHILDREN_PER_NODE: usize = 4_096;
/// Hard bound for semantic relationships exported by one node.
pub const MAX_SEMANTIC_RELATIONSHIPS_PER_NODE: usize = 256;

/// Coordinate space used by neutral semantic geometry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SemanticCoordinateSpace {
    /// Logical coordinates relative to the live view's content origin.
    ViewLogical,
}

/// Bounds and local transform computed by the authoritative layout pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SemanticNodeGeometry {
    bounds: RectF,
    transform: Transform2D,
    coordinate_space: SemanticCoordinateSpace,
}

impl SemanticNodeGeometry {
    pub fn new(
        bounds: RectF,
        transform: Transform2D,
        coordinate_space: SemanticCoordinateSpace,
    ) -> Result<Self, SemanticTreeError> {
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
        {
            return Err(SemanticTreeError::NonFiniteGeometry);
        }
        if bounds.width < 0.0 || bounds.height < 0.0 {
            return Err(SemanticTreeError::NegativeGeometryExtent);
        }
        if !transform.translation.x.is_finite()
            || !transform.translation.y.is_finite()
            || !transform.scale.x.is_finite()
            || !transform.scale.y.is_finite()
        {
            return Err(SemanticTreeError::NonFiniteTransform);
        }
        Ok(Self {
            bounds,
            transform,
            coordinate_space,
        })
    }

    pub const fn view_logical(bounds: RectF) -> Result<Self, SemanticTreeError> {
        if !bounds.x.is_finite()
            || !bounds.y.is_finite()
            || !bounds.width.is_finite()
            || !bounds.height.is_finite()
        {
            return Err(SemanticTreeError::NonFiniteGeometry);
        }
        if bounds.width < 0.0 || bounds.height < 0.0 {
            return Err(SemanticTreeError::NegativeGeometryExtent);
        }
        Ok(Self {
            bounds,
            transform: Transform2D {
                translation: crate::foundation::PointF { x: 0.0, y: 0.0 },
                scale: crate::foundation::PointF { x: 1.0, y: 1.0 },
                rotation: 0.0,
                origin: crate::foundation::PointF { x: 0.0, y: 0.0 },
            },
            coordinate_space: SemanticCoordinateSpace::ViewLogical,
        })
    }

    pub const fn bounds(self) -> RectF {
        self.bounds
    }

    pub const fn transform(self) -> Transform2D {
        self.transform
    }

    pub const fn coordinate_space(self) -> SemanticCoordinateSpace {
        self.coordinate_space
    }
}

/// One resolved string copied into an immutable semantic publication.
///
/// Debug output reports only identity and size so accessible names, descriptions, and values do
/// not become diagnostic plaintext.
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedSemanticString {
    id: StringId,
    value: Arc<str>,
}

impl ResolvedSemanticString {
    pub fn new(id: StringId, value: impl Into<Arc<str>>) -> Result<Self, SemanticTreeError> {
        let value = value.into();
        if value.len() > MAX_SEMANTIC_STRING_BYTES {
            return Err(SemanticTreeError::StringTooLarge { id });
        }
        Ok(Self { id, value })
    }

    pub const fn id(&self) -> StringId {
        self.id
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn byte_len(&self) -> usize {
        self.value.len()
    }
}

impl fmt::Debug for ResolvedSemanticString {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResolvedSemanticString")
            .field("id", &self.id)
            .field("byte_len", &self.value.len())
            .finish_non_exhaustive()
    }
}

/// One exported semantic node with stable mounted identity and authoritative geometry.
#[derive(Clone, Debug, PartialEq)]
pub struct SemanticTreeNode {
    id: SemanticNodeId,
    parent: Option<SemanticNodeId>,
    children: Arc<[SemanticNodeId]>,
    semantics: SemanticNode,
    geometry: SemanticNodeGeometry,
}

impl SemanticTreeNode {
    pub fn new(
        id: SemanticNodeId,
        parent: Option<SemanticNodeId>,
        children: Vec<SemanticNodeId>,
        semantics: SemanticNode,
        geometry: SemanticNodeGeometry,
    ) -> Result<Self, SemanticTreeError> {
        semantics
            .validate(id)
            .map_err(SemanticTreeError::InvalidSemanticInput)?;
        if semantics.participation != SemanticParticipation::Node {
            return Err(SemanticTreeError::UnresolvedParticipation { node: id });
        }
        if semantics.relationships.len() > MAX_SEMANTIC_RELATIONSHIPS_PER_NODE {
            return Err(SemanticTreeError::RelationshipLimitExceeded { node: id });
        }
        if children.len() > MAX_SEMANTIC_CHILDREN_PER_NODE {
            return Err(SemanticTreeError::ChildLimitExceeded { node: id });
        }
        if parent == Some(id) {
            return Err(SemanticTreeError::SelfParent { node: id });
        }
        for (index, child) in children.iter().copied().enumerate() {
            if child == id {
                return Err(SemanticTreeError::SelfChild { node: id });
            }
            if children[..index].contains(&child) {
                return Err(SemanticTreeError::DuplicateChild { parent: id, child });
            }
        }
        Ok(Self {
            id,
            parent,
            children: children.into(),
            semantics,
            geometry,
        })
    }

    pub const fn id(&self) -> SemanticNodeId {
        self.id
    }

    pub const fn parent(&self) -> Option<SemanticNodeId> {
        self.parent
    }

    pub fn children(&self) -> &[SemanticNodeId] {
        &self.children
    }

    pub const fn semantics(&self) -> &SemanticNode {
        &self.semantics
    }

    pub const fn geometry(&self) -> SemanticNodeGeometry {
        self.geometry
    }
}

//! Platform-neutral system-notification admission and response events.
//!
//! Content and reply text are bounded and redacted from diagnostics. Stable notification,
//! revision, and action identities make updates, removals, and responses exact. This module does
//! not schedule delivery, invoke an action, retain a native notification object, or own a callback,
//! queue, executor, thread, timer, or event loop.

use std::error::Error;
use std::fmt;
use std::num::{NonZeroU8, NonZeroU32, NonZeroU64};
use std::rc::Rc;
use std::sync::Arc;

use super::ServiceKey;
use crate::platform::contracts::{
    CapabilityDescriptor, PermissionState, RequestAdmission, Support, UserGestureGrantHandle,
};

pub const MAX_NOTIFICATION_TITLE_BYTES: usize = 256;
pub const MAX_NOTIFICATION_BODY_BYTES: usize = 4 * 1_024;
pub const MAX_NOTIFICATION_ACTION_LABEL_BYTES: usize = 256;
pub const MAX_NOTIFICATION_REPLY_BYTES: usize = 4 * 1_024;
pub const MAX_NOTIFICATION_ACTIONS: usize = 16;
pub const MAX_NOTIFICATION_BADGE_COUNT: u32 = 999_999;

macro_rules! define_notification_id {
    ($name:ident, $description:literal) => {
        #[doc = $description]
        #[repr(transparent)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            pub const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            pub const fn from_raw(value: u64) -> Option<Self> {
                match NonZeroU64::new(value) {
                    Some(value) => Some(Self(value)),
                    None => None,
                }
            }

            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.get().fmt(formatter)
            }
        }
    };
}

define_notification_id!(
    NotificationId,
    "Stable caller-owned identity for one system notification."
);
define_notification_id!(
    NotificationActionId,
    "Stable identity for one advertised notification action."
);

/// Monotonic revision of one [`NotificationId`] history.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NotificationRevision(NonZeroU64);

impl NotificationRevision {
    pub const INITIAL: Self = Self(NonZeroU64::MIN);

    pub const fn new(revision: NonZeroU64) -> Self {
        Self(revision)
    }

    pub const fn from_raw(revision: u64) -> Option<Self> {
        match NonZeroU64::new(revision) {
            Some(revision) => Some(Self(revision)),
            None => None,
        }
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub const fn checked_next(self) -> Option<Self> {
        match self.get().checked_add(1) {
            Some(revision) => Self::from_raw(revision),
            None => None,
        }
    }
}

impl fmt::Display for NotificationRevision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.get().fmt(formatter)
    }
}

/// Exact identity of one notification publication.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationSnapshotId {
    notification: NotificationId,
    revision: NotificationRevision,
}

impl NotificationSnapshotId {
    pub const fn new(notification: NotificationId, revision: NotificationRevision) -> Self {
        Self {
            notification,
            revision,
        }
    }

    pub const fn notification(self) -> NotificationId {
        self.notification
    }

    pub const fn revision(self) -> NotificationRevision {
        self.revision
    }
}

macro_rules! define_redacted_text {
    ($name:ident, $maximum:ident, $description:literal) => {
        #[doc = $description]
        #[derive(Clone, PartialEq, Eq, Hash)]
        pub struct $name(Arc<str>);

        impl $name {
            pub fn new(value: impl AsRef<str>) -> Result<Self, NotificationTextError> {
                let value = value.as_ref();
                validate_notification_text(value, $maximum)?;
                Ok(Self(Arc::from(value)))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn byte_len(&self) -> usize {
                self.0.len()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter
                    .debug_struct(stringify!($name))
                    .field("byte_len", &self.byte_len())
                    .field("redacted", &true)
                    .finish()
            }
        }
    };
}

define_redacted_text!(
    NotificationTitle,
    MAX_NOTIFICATION_TITLE_BYTES,
    "Bounded notification title omitted from diagnostics."
);
define_redacted_text!(
    NotificationBody,
    MAX_NOTIFICATION_BODY_BYTES,
    "Bounded optional notification body omitted from diagnostics."
);
define_redacted_text!(
    NotificationActionLabel,
    MAX_NOTIFICATION_ACTION_LABEL_BYTES,
    "Bounded visible action label omitted from diagnostics."
);
define_redacted_text!(
    NotificationReply,
    MAX_NOTIFICATION_REPLY_BYTES,
    "Bounded inline reply text omitted from diagnostics."
);

fn validate_notification_text(
    value: &str,
    maximum_bytes: usize,
) -> Result<(), NotificationTextError> {
    if value.trim().is_empty() {
        return Err(NotificationTextError::Empty);
    }
    if value.len() > maximum_bytes {
        return Err(NotificationTextError::TooLong {
            byte_len: value.len(),
            maximum_bytes,
        });
    }
    if value.chars().any(char::is_control) {
        return Err(NotificationTextError::ControlCharacter);
    }
    Ok(())
}

/// Invalid notification content or reply text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationTextError {
    Empty,
    TooLong {
        byte_len: usize,
        maximum_bytes: usize,
    },
    ControlCharacter,
}

impl fmt::Display for NotificationTextError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("notification text is empty"),
            Self::TooLong {
                byte_len,
                maximum_bytes,
            } => write!(
                formatter,
                "notification text contains {byte_len} bytes; maximum is {maximum_bytes}"
            ),
            Self::ControlCharacter => {
                formatter.write_str("notification text contains a control character")
            }
        }
    }
}

impl Error for NotificationTextError {}

/// Portable delivery urgency requested from the native service.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NotificationPriority {
    Low,
    #[default]
    Normal,
    High,
    Critical,
}

/// Content sensitivity retained for platform lock-screen/privacy policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NotificationPrivacy {
    #[default]
    Public,
    Sensitive,
    Secret,
}

/// Semantic behavior of one notification action.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationActionKind {
    Default,
    Open,
    Reply,
    Dismiss,
    Custom,
}

/// Bounded action metadata; it contains no callback or command object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationAction {
    id: NotificationActionId,
    kind: NotificationActionKind,
    label: Option<NotificationActionLabel>,
}

impl NotificationAction {
    pub fn new(
        id: NotificationActionId,
        kind: NotificationActionKind,
        label: Option<NotificationActionLabel>,
    ) -> Result<Self, NotificationActionError> {
        match (kind, label.is_some()) {
            (NotificationActionKind::Default, true) => {
                return Err(NotificationActionError::DefaultActionHasLabel);
            }
            (NotificationActionKind::Default, false) => {}
            (_, false) => return Err(NotificationActionError::VisibleActionMissingLabel),
            (_, true) => {}
        }
        Ok(Self { id, kind, label })
    }

    pub const fn id(&self) -> NotificationActionId {
        self.id
    }

    pub const fn kind(&self) -> NotificationActionKind {
        self.kind
    }

    pub const fn label(&self) -> Option<&NotificationActionLabel> {
        self.label.as_ref()
    }
}

/// Invalid action-kind and presentation relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationActionError {
    DefaultActionHasLabel,
    VisibleActionMissingLabel,
}

impl fmt::Display for NotificationActionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::DefaultActionHasLabel => "default notification action must not supply a label",
            Self::VisibleActionMissingLabel => "visible notification action requires a label",
        })
    }
}

impl Error for NotificationActionError {}

/// Complete bounded notification content at one exact identity and revision.
#[derive(Clone, PartialEq, Eq)]
pub struct NotificationDescriptor {
    snapshot: NotificationSnapshotId,
    title: NotificationTitle,
    body: Option<NotificationBody>,
    priority: NotificationPriority,
    privacy: NotificationPrivacy,
    actions: Arc<[NotificationAction]>,
}

impl NotificationDescriptor {
    pub fn new(
        snapshot: NotificationSnapshotId,
        title: NotificationTitle,
        body: Option<NotificationBody>,
        priority: NotificationPriority,
        privacy: NotificationPrivacy,
        actions: Vec<NotificationAction>,
    ) -> Result<Self, NotificationDescriptorError> {
        if actions.len() > MAX_NOTIFICATION_ACTIONS {
            return Err(NotificationDescriptorError::TooManyActions {
                supplied: actions.len(),
                maximum: MAX_NOTIFICATION_ACTIONS,
            });
        }
        let mut has_default = false;
        let mut has_dismiss = false;
        for (index, action) in actions.iter().enumerate() {
            if actions[..index]
                .iter()
                .any(|previous| previous.id == action.id)
            {
                return Err(NotificationDescriptorError::DuplicateAction { action: action.id });
            }
            match action.kind {
                NotificationActionKind::Default if has_default => {
                    return Err(NotificationDescriptorError::DuplicateDefaultAction);
                }
                NotificationActionKind::Default => has_default = true,
                NotificationActionKind::Dismiss if has_dismiss => {
                    return Err(NotificationDescriptorError::DuplicateDismissAction);
                }
                NotificationActionKind::Dismiss => has_dismiss = true,
                NotificationActionKind::Open
                | NotificationActionKind::Reply
                | NotificationActionKind::Custom => {}
            }
        }
        Ok(Self {
            snapshot,
            title,
            body,
            priority,
            privacy,
            actions: actions.into(),
        })
    }

    pub const fn snapshot(&self) -> NotificationSnapshotId {
        self.snapshot
    }

    pub const fn title(&self) -> &NotificationTitle {
        &self.title
    }

    pub const fn body(&self) -> Option<&NotificationBody> {
        self.body.as_ref()
    }

    pub const fn priority(&self) -> NotificationPriority {
        self.priority
    }

    pub const fn privacy(&self) -> NotificationPrivacy {
        self.privacy
    }

    pub fn actions(&self) -> &[NotificationAction] {
        &self.actions
    }

    pub fn action(&self, id: NotificationActionId) -> Option<&NotificationAction> {
        self.actions.iter().find(|action| action.id == id)
    }

    pub fn reply_action_count(&self) -> usize {
        self.actions
            .iter()
            .filter(|action| action.kind == NotificationActionKind::Reply)
            .count()
    }
}

impl fmt::Debug for NotificationDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotificationDescriptor")
            .field("snapshot", &self.snapshot)
            .field("title", &self.title)
            .field("has_body", &self.body.is_some())
            .field("priority", &self.priority)
            .field("privacy", &self.privacy)
            .field("action_count", &self.actions.len())
            .finish_non_exhaustive()
    }
}

/// Invalid complete notification metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationDescriptorError {
    TooManyActions { supplied: usize, maximum: usize },
    DuplicateAction { action: NotificationActionId },
    DuplicateDefaultAction,
    DuplicateDismissAction,
}

impl fmt::Display for NotificationDescriptorError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TooManyActions { .. } => "notification exceeds the action-count bound",
            Self::DuplicateAction { .. } => "notification repeats an action identity",
            Self::DuplicateDefaultAction => "notification contains multiple default actions",
            Self::DuplicateDismissAction => "notification contains multiple dismiss actions",
        })
    }
}

impl Error for NotificationDescriptorError {}

/// Initial or exact-successor notification publication.
#[derive(Clone, PartialEq, Eq)]
pub struct NotificationPublicationRequest {
    previous: Option<NotificationSnapshotId>,
    descriptor: NotificationDescriptor,
}

impl NotificationPublicationRequest {
    pub fn initial(
        descriptor: NotificationDescriptor,
    ) -> Result<Self, NotificationPublicationError> {
        if descriptor.snapshot.revision != NotificationRevision::INITIAL {
            return Err(NotificationPublicationError::InitialRevisionRequired {
                supplied: descriptor.snapshot.revision,
            });
        }
        Ok(Self {
            previous: None,
            descriptor,
        })
    }

    pub fn advance(
        previous: NotificationSnapshotId,
        descriptor: NotificationDescriptor,
    ) -> Result<Self, NotificationPublicationError> {
        if previous.notification != descriptor.snapshot.notification {
            return Err(NotificationPublicationError::NotificationMismatch {
                previous: previous.notification,
                current: descriptor.snapshot.notification,
            });
        }
        let Some(expected) = previous.revision.checked_next() else {
            return Err(NotificationPublicationError::RevisionExhausted);
        };
        if descriptor.snapshot.revision != expected {
            return Err(NotificationPublicationError::RevisionNotNext {
                previous: previous.revision,
                current: descriptor.snapshot.revision,
            });
        }
        Ok(Self {
            previous: Some(previous),
            descriptor,
        })
    }

    pub const fn previous(&self) -> Option<NotificationSnapshotId> {
        self.previous
    }

    pub const fn descriptor(&self) -> &NotificationDescriptor {
        &self.descriptor
    }

    pub fn into_descriptor(self) -> NotificationDescriptor {
        self.descriptor
    }
}

impl fmt::Debug for NotificationPublicationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotificationPublicationRequest")
            .field("previous", &self.previous)
            .field("current", &self.descriptor.snapshot)
            .field("priority", &self.descriptor.priority)
            .field("privacy", &self.descriptor.privacy)
            .field("action_count", &self.descriptor.actions.len())
            .finish_non_exhaustive()
    }
}

/// Invalid notification identity/revision publication relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationPublicationError {
    InitialRevisionRequired {
        supplied: NotificationRevision,
    },
    NotificationMismatch {
        previous: NotificationId,
        current: NotificationId,
    },
    RevisionNotNext {
        previous: NotificationRevision,
        current: NotificationRevision,
    },
    RevisionExhausted,
}

impl fmt::Display for NotificationPublicationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InitialRevisionRequired { .. } => {
                "initial notification publication must use the initial revision"
            }
            Self::NotificationMismatch { .. } => {
                "notification update changes stable notification identity"
            }
            Self::RevisionNotNext { .. } => {
                "notification update revision is not the exact successor"
            }
            Self::RevisionExhausted => "notification revision is exhausted",
        })
    }
}

impl Error for NotificationPublicationError {}

/// Applied notification publication metadata without content.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationPublicationApplied {
    snapshot: NotificationSnapshotId,
}

impl NotificationPublicationApplied {
    pub const fn from_request(request: &NotificationPublicationRequest) -> Self {
        Self {
            snapshot: request.descriptor.snapshot,
        }
    }

    pub const fn snapshot(self) -> NotificationSnapshotId {
        self.snapshot
    }
}

/// Exact-current notification removal intention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationRemovalRequest {
    expected: NotificationSnapshotId,
}

impl NotificationRemovalRequest {
    pub const fn new(expected: NotificationSnapshotId) -> Self {
        Self { expected }
    }

    pub const fn expected(self) -> NotificationSnapshotId {
        self.expected
    }
}

/// Applied notification removal metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationRemovalApplied {
    removed: NotificationSnapshotId,
}

impl NotificationRemovalApplied {
    pub const fn from_request(request: NotificationRemovalRequest) -> Self {
        Self {
            removed: request.expected,
        }
    }

    pub const fn removed(self) -> NotificationSnapshotId {
        self.removed
    }
}

/// Numeric application-icon badge intent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum NotificationBadge {
    #[default]
    Clear,
    Count(NonZeroU32),
}

impl NotificationBadge {
    pub const fn count(count: NonZeroU32) -> Result<Self, NotificationBadgeError> {
        if count.get() > MAX_NOTIFICATION_BADGE_COUNT {
            return Err(NotificationBadgeError::CountTooLarge {
                supplied: count.get(),
                maximum: MAX_NOTIFICATION_BADGE_COUNT,
            });
        }
        Ok(Self::Count(count))
    }

    pub const fn value(self) -> Option<NonZeroU32> {
        match self {
            Self::Clear => None,
            Self::Count(count) => Some(count),
        }
    }
}

/// Invalid badge count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationBadgeError {
    CountTooLarge { supplied: u32, maximum: u32 },
}

impl fmt::Display for NotificationBadgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("notification badge count exceeds the neutral hard bound")
    }
}

impl Error for NotificationBadgeError {}

/// Badge update intention.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationBadgeRequest {
    badge: NotificationBadge,
}

impl NotificationBadgeRequest {
    pub const fn new(badge: NotificationBadge) -> Self {
        Self { badge }
    }

    pub const fn badge(self) -> NotificationBadge {
        self.badge
    }
}

/// Applied badge metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationBadgeApplied {
    badge: NotificationBadge,
}

impl NotificationBadgeApplied {
    pub const fn from_request(request: NotificationBadgeRequest) -> Self {
        Self {
            badge: request.badge,
        }
    }

    pub const fn badge(self) -> NotificationBadge {
        self.badge
    }
}

/// Authorization dimensions requested from the native notification service.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationAuthorizationOptions {
    alerts: bool,
    badges: bool,
    sounds: bool,
    critical_alerts: bool,
}

impl NotificationAuthorizationOptions {
    pub const fn new(
        alerts: bool,
        badges: bool,
        sounds: bool,
        critical_alerts: bool,
    ) -> Result<Self, NotificationAuthorizationOptionsError> {
        if !alerts && !badges && !sounds && !critical_alerts {
            return Err(NotificationAuthorizationOptionsError::Empty);
        }
        if critical_alerts && !alerts {
            return Err(NotificationAuthorizationOptionsError::CriticalAlertsRequireAlerts);
        }
        Ok(Self {
            alerts,
            badges,
            sounds,
            critical_alerts,
        })
    }

    pub const fn alerts(self) -> bool {
        self.alerts
    }

    pub const fn badges(self) -> bool {
        self.badges
    }

    pub const fn sounds(self) -> bool {
        self.sounds
    }

    pub const fn critical_alerts(self) -> bool {
        self.critical_alerts
    }
}

/// Invalid authorization option relationship.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationAuthorizationOptionsError {
    Empty,
    CriticalAlertsRequireAlerts,
}

impl fmt::Display for NotificationAuthorizationOptionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "notification authorization requests no capability",
            Self::CriticalAlertsRequireAlerts => {
                "critical notification authorization requires alerts"
            }
        })
    }
}

impl Error for NotificationAuthorizationOptionsError {}

/// Authorization prompt intention with optional opaque recent-gesture evidence.
pub struct NotificationAuthorizationRequest {
    options: NotificationAuthorizationOptions,
    user_gesture: Option<UserGestureGrantHandle>,
}

impl NotificationAuthorizationRequest {
    pub const fn new(options: NotificationAuthorizationOptions) -> Self {
        Self {
            options,
            user_gesture: None,
        }
    }

    pub fn with_user_gesture(
        options: NotificationAuthorizationOptions,
        user_gesture: UserGestureGrantHandle,
    ) -> Self {
        Self {
            options,
            user_gesture: Some(user_gesture),
        }
    }

    pub const fn options(&self) -> NotificationAuthorizationOptions {
        self.options
    }

    pub const fn has_user_gesture(&self) -> bool {
        self.user_gesture.is_some()
    }

    pub fn into_parts(
        self,
    ) -> (
        NotificationAuthorizationOptions,
        Option<UserGestureGrantHandle>,
    ) {
        (self.options, self.user_gesture)
    }
}

impl fmt::Debug for NotificationAuthorizationRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NotificationAuthorizationRequest")
            .field("options", &self.options)
            .field("has_user_gesture", &self.user_gesture.is_some())
            .finish_non_exhaustive()
    }
}

/// Observed authorization result returned by a completed prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationAuthorizationApplied {
    permission: PermissionState,
}

impl NotificationAuthorizationApplied {
    pub const fn new(permission: PermissionState) -> Self {
        Self { permission }
    }

    pub const fn permission(self) -> PermissionState {
        self.permission
    }
}

/// Independently discoverable notification features.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct NotificationOperations {
    authorization_request: bool,
    publish: bool,
    update: bool,
    remove: bool,
    actions: bool,
    inline_reply: bool,
    badges: bool,
    response_events: bool,
}

impl NotificationOperations {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        authorization_request: bool,
        publish: bool,
        update: bool,
        remove: bool,
        actions: bool,
        inline_reply: bool,
        badges: bool,
        response_events: bool,
    ) -> Self {
        Self {
            authorization_request,
            publish,
            update,
            remove,
            actions,
            inline_reply,
            badges,
            response_events,
        }
    }

    pub const fn supports_authorization_request(self) -> bool {
        self.authorization_request
    }

    pub const fn supports_publish(self) -> bool {
        self.publish
    }

    pub const fn supports_update(self) -> bool {
        self.update
    }

    pub const fn supports_remove(self) -> bool {
        self.remove
    }

    pub const fn supports_actions(self) -> bool {
        self.actions
    }

    pub const fn supports_inline_reply(self) -> bool {
        self.inline_reply
    }

    pub const fn supports_badges(self) -> bool {
        self.badges
    }

    pub const fn supports_response_events(self) -> bool {
        self.response_events
    }
}

/// Adapter-advertised bounds capped by the neutral hard limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NotificationLimits {
    maximum_actions: NonZeroU8,
    maximum_body_bytes: NonZeroU32,
    maximum_reply_bytes: NonZeroU32,
    maximum_badge_count: NonZeroU32,
}

impl NotificationLimits {
    pub const fn new(
        maximum_actions: NonZeroU8,
        maximum_body_bytes: NonZeroU32,
        maximum_reply_bytes: NonZeroU32,
        maximum_badge_count: NonZeroU32,
    ) -> Result<Self, NotificationLimitError> {
        if maximum_actions.get() as usize > MAX_NOTIFICATION_ACTIONS {
            return Err(NotificationLimitError::ActionLimitTooLarge);
        }
        if maximum_body_bytes.get() as usize > MAX_NOTIFICATION_BODY_BYTES {
            return Err(NotificationLimitError::BodyLimitTooLarge);
        }
        if maximum_reply_bytes.get() as usize > MAX_NOTIFICATION_REPLY_BYTES {
            return Err(NotificationLimitError::ReplyLimitTooLarge);
        }
        if maximum_badge_count.get() > MAX_NOTIFICATION_BADGE_COUNT {
            return Err(NotificationLimitError::BadgeLimitTooLarge);
        }
        Ok(Self {
            maximum_actions,
            maximum_body_bytes,
            maximum_reply_bytes,
            maximum_badge_count,
        })
    }

    pub const fn maximum_actions(self) -> NonZeroU8 {
        self.maximum_actions
    }

    pub const fn maximum_body_bytes(self) -> NonZeroU32 {
        self.maximum_body_bytes
    }

    pub const fn maximum_reply_bytes(self) -> NonZeroU32 {
        self.maximum_reply_bytes
    }

    pub const fn maximum_badge_count(self) -> NonZeroU32 {
        self.maximum_badge_count
    }
}

impl Default for NotificationLimits {
    fn default() -> Self {
        Self {
            maximum_actions: NonZeroU8::new(MAX_NOTIFICATION_ACTIONS as u8)
                .expect("notification action hard bound is nonzero"),
            maximum_body_bytes: NonZeroU32::new(MAX_NOTIFICATION_BODY_BYTES as u32)
                .expect("notification body hard bound is nonzero"),
            maximum_reply_bytes: NonZeroU32::new(MAX_NOTIFICATION_REPLY_BYTES as u32)
                .expect("notification reply hard bound is nonzero"),
            maximum_badge_count: NonZeroU32::new(MAX_NOTIFICATION_BADGE_COUNT)
                .expect("notification badge hard bound is nonzero"),
        }
    }
}

/// Invalid host-advertised notification limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationLimitError {
    ActionLimitTooLarge,
    BodyLimitTooLarge,
    ReplyLimitTooLarge,
    BadgeLimitTooLarge,
}

impl fmt::Display for NotificationLimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ActionLimitTooLarge => "notification action limit exceeds the hard bound",
            Self::BodyLimitTooLarge => "notification body limit exceeds the hard bound",
            Self::ReplyLimitTooLarge => "notification reply limit exceeds the hard bound",
            Self::BadgeLimitTooLarge => "notification badge limit exceeds the hard bound",
        })
    }
}

impl Error for NotificationLimitError {}

pub type NotificationCapability = CapabilityDescriptor<NotificationOperations, NotificationLimits>;

/// Immediate rejection before a notification request is admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationAdmissionError {
    UnsupportedOperation,
    PermissionDenied,
    AuthorizationRequired,
    UserGestureRequired,
    InvalidUserGesture,
    ActionsUnsupported,
    InlineReplyUnsupported,
    BodyExceedsCapability,
    ActionsExceedCapability,
    ReplyExceedsCapability,
    BadgeExceedsCapability,
    NotificationUnavailable {
        notification: NotificationId,
    },
    RevisionMismatch {
        expected: NotificationSnapshotId,
        observed: Option<NotificationSnapshotId>,
    },
    CapabilityChanged,
    CapacityExceeded,
}

impl fmt::Display for NotificationAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::UnsupportedOperation => "notification operation is unsupported",
            Self::PermissionDenied => "notification permission is denied",
            Self::AuthorizationRequired => "notification authorization is required",
            Self::UserGestureRequired => "notification authorization requires a user gesture",
            Self::InvalidUserGesture => "notification authorization gesture is invalid",
            Self::ActionsUnsupported => "notification actions are unsupported",
            Self::InlineReplyUnsupported => "notification inline reply is unsupported",
            Self::BodyExceedsCapability => "notification body exceeds capability",
            Self::ActionsExceedCapability => "notification action count exceeds capability",
            Self::ReplyExceedsCapability => "notification reply exceeds capability",
            Self::BadgeExceedsCapability => "notification badge exceeds capability",
            Self::NotificationUnavailable { .. } => "notification identity is unavailable",
            Self::RevisionMismatch { .. } => "notification request cites a stale revision",
            Self::CapabilityChanged => "notification capability changed before admission",
            Self::CapacityExceeded => "notification request admission capacity was exceeded",
        })
    }
}

impl Error for NotificationAdmissionError {}

pub type NotificationAuthorizationAdmission =
    RequestAdmission<NotificationAuthorizationApplied, NotificationAdmissionError>;
pub type NotificationPublicationAdmission =
    RequestAdmission<NotificationPublicationApplied, NotificationAdmissionError>;
pub type NotificationRemovalAdmission =
    RequestAdmission<NotificationRemovalApplied, NotificationAdmissionError>;
pub type NotificationBadgeAdmission =
    RequestAdmission<NotificationBadgeApplied, NotificationAdmissionError>;

/// Native source classification of a notification response.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationResponseSource {
    Body,
    Action,
    InlineReply,
    DismissedByUser,
    DismissedBySystem,
    Expired,
}

/// Native response candidate citing one exact notification snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationResponseRequest {
    snapshot: NotificationSnapshotId,
    action: Option<NotificationActionId>,
    source: NotificationResponseSource,
    reply: Option<NotificationReply>,
}

impl NotificationResponseRequest {
    pub const fn new(
        snapshot: NotificationSnapshotId,
        action: Option<NotificationActionId>,
        source: NotificationResponseSource,
        reply: Option<NotificationReply>,
    ) -> Self {
        Self {
            snapshot,
            action,
            source,
            reply,
        }
    }

    pub const fn snapshot(&self) -> NotificationSnapshotId {
        self.snapshot
    }

    pub const fn action(&self) -> Option<NotificationActionId> {
        self.action
    }

    pub const fn source(&self) -> NotificationResponseSource {
        self.source
    }

    pub const fn reply(&self) -> Option<&NotificationReply> {
        self.reply.as_ref()
    }
}

/// Validated source-qualified notification response event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationResponseEvent {
    snapshot: NotificationSnapshotId,
    action: Option<NotificationActionId>,
    source: NotificationResponseSource,
    reply: Option<NotificationReply>,
}

impl NotificationResponseEvent {
    pub fn admit(
        current: &NotificationDescriptor,
        request: NotificationResponseRequest,
    ) -> NotificationResponseAdmission {
        if request.snapshot.notification != current.snapshot.notification {
            return Err(NotificationResponseAdmissionError::NotificationMismatch {
                expected: current.snapshot.notification,
                observed: request.snapshot.notification,
            });
        }
        if request.snapshot.revision != current.snapshot.revision {
            return Err(NotificationResponseAdmissionError::StaleRevision {
                expected: current.snapshot.revision,
                observed: request.snapshot.revision,
            });
        }

        let action = match request.action {
            Some(id) => Some(
                current
                    .action(id)
                    .ok_or(NotificationResponseAdmissionError::UnknownAction { action: id })?,
            ),
            None => None,
        };
        match request.source {
            NotificationResponseSource::Body => {
                let Some(action) = action else {
                    return Err(NotificationResponseAdmissionError::ActionRequired);
                };
                if action.kind != NotificationActionKind::Default {
                    return Err(NotificationResponseAdmissionError::DefaultActionRequired);
                }
                if request.reply.is_some() {
                    return Err(NotificationResponseAdmissionError::UnexpectedReply);
                }
            }
            NotificationResponseSource::Action => {
                let Some(action) = action else {
                    return Err(NotificationResponseAdmissionError::ActionRequired);
                };
                if action.kind == NotificationActionKind::Default {
                    return Err(NotificationResponseAdmissionError::VisibleActionRequired);
                }
                if request.reply.is_some() {
                    return Err(NotificationResponseAdmissionError::UnexpectedReply);
                }
            }
            NotificationResponseSource::InlineReply => {
                let Some(action) = action else {
                    return Err(NotificationResponseAdmissionError::ActionRequired);
                };
                if action.kind != NotificationActionKind::Reply {
                    return Err(NotificationResponseAdmissionError::ReplyActionRequired);
                }
                if request.reply.is_none() {
                    return Err(NotificationResponseAdmissionError::ReplyRequired);
                }
            }
            NotificationResponseSource::DismissedByUser
            | NotificationResponseSource::DismissedBySystem
            | NotificationResponseSource::Expired => {
                if request.action.is_some() {
                    return Err(NotificationResponseAdmissionError::UnexpectedAction);
                }
                if request.reply.is_some() {
                    return Err(NotificationResponseAdmissionError::UnexpectedReply);
                }
            }
        }
        Ok(Self {
            snapshot: request.snapshot,
            action: request.action,
            source: request.source,
            reply: request.reply,
        })
    }

    pub const fn snapshot(&self) -> NotificationSnapshotId {
        self.snapshot
    }

    pub const fn action(&self) -> Option<NotificationActionId> {
        self.action
    }

    pub const fn source(&self) -> NotificationResponseSource {
        self.source
    }

    pub const fn reply(&self) -> Option<&NotificationReply> {
        self.reply.as_ref()
    }
}

/// Rejection before a native response becomes a portable event.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NotificationResponseAdmissionError {
    NotificationMismatch {
        expected: NotificationId,
        observed: NotificationId,
    },
    StaleRevision {
        expected: NotificationRevision,
        observed: NotificationRevision,
    },
    UnknownAction {
        action: NotificationActionId,
    },
    ActionRequired,
    UnexpectedAction,
    DefaultActionRequired,
    VisibleActionRequired,
    ReplyActionRequired,
    ReplyRequired,
    UnexpectedReply,
}

impl fmt::Display for NotificationResponseAdmissionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotificationMismatch { .. } => "notification response cites another notification",
            Self::StaleRevision { .. } => "notification response cites a stale revision",
            Self::UnknownAction { .. } => "notification response cites an unknown action",
            Self::ActionRequired => "notification response requires an action",
            Self::UnexpectedAction => "notification dismissal must not cite an action",
            Self::DefaultActionRequired => "notification body response requires the default action",
            Self::VisibleActionRequired => "notification action response requires a visible action",
            Self::ReplyActionRequired => "inline reply requires a reply action",
            Self::ReplyRequired => "inline reply contains no reply text",
            Self::UnexpectedReply => "notification response contains unexpected reply text",
        })
    }
}

impl Error for NotificationResponseAdmissionError {}

pub type NotificationResponseAdmission =
    Result<NotificationResponseEvent, NotificationResponseAdmissionError>;

/// Object-safe notification capability and request-admission boundary.
pub trait NotificationService {
    fn capability(&self) -> Support<NotificationCapability>;

    fn authorize(
        &self,
        request: NotificationAuthorizationRequest,
    ) -> NotificationAuthorizationAdmission;

    fn publish(&self, request: NotificationPublicationRequest) -> NotificationPublicationAdmission;

    fn remove(&self, request: NotificationRemovalRequest) -> NotificationRemovalAdmission;

    fn set_badge(&self, request: NotificationBadgeRequest) -> NotificationBadgeAdmission;
}

pub enum NotificationServiceKey {}

impl ServiceKey for NotificationServiceKey {
    type Handle = Rc<dyn NotificationService>;
}

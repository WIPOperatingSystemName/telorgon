use std::ffi::CStr;
use std::fmt;
use std::os::fd::{FromRawFd, OwnedFd};
use std::slice;

use crate::integrations::wayland::server::ffi::{wl_argument, wl_resource};
use crate::integrations::wayland::server::{ArgumentType, MessageSchema, ResourceRef};

/// Schema-checked view over one request decoded by libwayland.
pub struct IncomingRequest<'request> {
    message: &'request MessageSchema,
    arguments: &'request mut [wl_argument],
}

impl<'request> IncomingRequest<'request> {
    /// # Safety
    ///
    /// `arguments` must contain one libwayland-decoded entry for every schema argument and remain
    /// live and exclusively borrowed for `'request`.
    pub unsafe fn from_raw(
        message: &'request MessageSchema,
        arguments: *mut wl_argument,
    ) -> Result<Self, RequestDecodeError> {
        if message.arguments.len() > 128 || (arguments.is_null() && !message.arguments.is_empty()) {
            return Err(RequestDecodeError::MalformedArguments);
        }
        let arguments = unsafe { slice::from_raw_parts_mut(arguments, message.arguments.len()) };
        Ok(Self { message, arguments })
    }

    pub fn message(&self) -> &MessageSchema {
        self.message
    }

    pub fn int(&self, index: usize) -> Result<i32, RequestDecodeError> {
        self.require(index, ArgumentType::Int)?;
        Ok(unsafe { self.arguments[index].i })
    }

    pub fn uint(&self, index: usize) -> Result<u32, RequestDecodeError> {
        self.require(index, ArgumentType::Uint)?;
        Ok(unsafe { self.arguments[index].u })
    }

    pub fn fixed(&self, index: usize) -> Result<i32, RequestDecodeError> {
        self.require(index, ArgumentType::Fixed)?;
        Ok(unsafe { self.arguments[index].f })
    }

    pub fn string(&self, index: usize) -> Result<Option<&CStr>, RequestDecodeError> {
        self.require(index, ArgumentType::String)?;
        let pointer = unsafe { self.arguments[index].s };
        if pointer.is_null() {
            if self.message.arguments[index].allow_null {
                Ok(None)
            } else {
                Err(RequestDecodeError::UnexpectedNull)
            }
        } else {
            Ok(Some(unsafe { CStr::from_ptr(pointer) }))
        }
    }

    pub fn object(
        &self,
        index: usize,
    ) -> Result<Option<ResourceRef<'request>>, RequestDecodeError> {
        self.require(index, ArgumentType::Object)?;
        let pointer = unsafe { self.arguments[index].o };
        if pointer.is_null() && !self.message.arguments[index].allow_null {
            return Err(RequestDecodeError::UnexpectedNull);
        }
        Ok(unsafe { ResourceRef::from_raw(pointer) })
    }

    pub fn new_id(&self, index: usize) -> Result<u32, RequestDecodeError> {
        self.require(index, ArgumentType::NewId)?;
        let id = unsafe { self.arguments[index].n };
        if id == 0 {
            Err(RequestDecodeError::InvalidNewId)
        } else {
            Ok(id)
        }
    }

    /// Transfers ownership of an incoming file descriptor to the caller exactly once.
    pub fn take_fd(&mut self, index: usize) -> Result<OwnedFd, RequestDecodeError> {
        self.require(index, ArgumentType::Fd)?;
        let fd = unsafe { self.arguments[index].h };
        if fd < 0 {
            return Err(RequestDecodeError::InvalidFileDescriptor);
        }
        self.arguments[index].h = -1;
        Ok(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    pub fn array(&self, index: usize) -> Result<&[u8], RequestDecodeError> {
        self.require(index, ArgumentType::Array)?;
        let array = unsafe { self.arguments[index].a };
        if array.is_null() {
            return Err(RequestDecodeError::UnexpectedNull);
        }
        let array = unsafe { &*array };
        if array.data.is_null() && array.size != 0 {
            return Err(RequestDecodeError::MalformedArguments);
        }
        Ok(unsafe { slice::from_raw_parts(array.data.cast::<u8>(), array.size) })
    }

    fn require(&self, index: usize, expected: ArgumentType) -> Result<(), RequestDecodeError> {
        let actual = self
            .message
            .arguments
            .get(index)
            .map(|argument| argument.argument_type)
            .ok_or(RequestDecodeError::IndexOutOfBounds)?;
        if actual == expected {
            Ok(())
        } else {
            Err(RequestDecodeError::WrongType)
        }
    }
}

impl Drop for IncomingRequest<'_> {
    fn drop(&mut self) {
        for (schema, argument) in self.message.arguments.iter().zip(self.arguments.iter_mut()) {
            if schema.argument_type == ArgumentType::Fd {
                let fd = unsafe { argument.h };
                if fd >= 0 {
                    drop(unsafe { OwnedFd::from_raw_fd(fd) });
                    argument.h = -1;
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestDecodeError {
    MalformedArguments,
    IndexOutOfBounds,
    WrongType,
    UnexpectedNull,
    InvalidNewId,
    InvalidFileDescriptor,
}

impl fmt::Display for RequestDecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Wayland request argument decode failed: {self:?}"
        )
    }
}

impl std::error::Error for RequestDecodeError {}

const _: fn(*mut wl_resource) = |_| {};

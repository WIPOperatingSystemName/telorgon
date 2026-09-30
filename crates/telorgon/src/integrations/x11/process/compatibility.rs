//! Host ABI checks belong at helper preparation, not offline archive validation.
use crate::integrations::x11::{Error, Result, payload_format};
use std::ffi::CStr;

pub(super) fn ensure_requirement(required: &str) -> Result<()> {
    // SAFETY: glibc returns a process-lifetime, NUL-terminated version string.
    let pointer = unsafe { libc::gnu_get_libc_version() };
    if pointer.is_null() {
        return Err(Error("cannot identify the installed glibc version".into()));
    }
    let installed = unsafe { CStr::from_ptr(pointer) }
        .to_str()
        .map_err(|_| Error("invalid installed glibc version".into()))?;
    compare(required, installed)
}

fn compare(required: &str, installed: &str) -> Result<()> {
    let minimum = payload_format::glibc_version(required)
        .ok_or_else(|| Error("invalid Xwayland glibc requirement".into()))?;
    let current = payload_format::glibc_version(installed)
        .ok_or_else(|| Error("invalid installed glibc version".into()))?;
    if current < minimum {
        return Err(Error(format!(
            "Xwayland payload requires glibc {required}; installed glibc is {installed}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_numeric_versions_and_historical_patch_requirements() {
        for (required, installed) in [
            ("2.39", "2.43"),
            ("2.43", "2.43"),
            ("2.9", "2.10"),
            ("2.2.5", "2.43"),
            ("2.2.5", "2.2.5"),
        ] {
            assert!(
                compare(required, installed).is_ok(),
                "{required} / {installed}"
            );
        }
        for (required, installed) in [("2.43", "2.39"), ("2.10", "2.9"), ("2.2.5", "2.2")] {
            let error = compare(required, installed).unwrap_err().to_string();
            assert!(error.contains(required) && error.contains(installed));
        }
        assert!(compare("2.043", "2.43").is_err());
        assert!(compare("2.43", "not-a-version").is_err());
    }

    #[test]
    fn identifies_the_installed_gnu_runtime_without_launching_a_helper() {
        ensure_requirement("2.2.5").unwrap();
    }
}

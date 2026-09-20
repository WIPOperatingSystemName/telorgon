use super::*;

#[test]
fn keyboard_names_preserve_defaults_and_reject_nul_in_every_field() {
    let defaults = LinuxShellConfig::default();
    assert_eq!(defaults.keyboard, KeyboardConfig::default());
    defaults.validate().unwrap();
    for field in 0..5 {
        let mut config = LinuxShellConfig::default();
        let names = &mut config.keyboard;
        let target = match field {
            0 => &mut names.rules,
            1 => &mut names.model,
            2 => &mut names.layout,
            3 => &mut names.variant,
            _ => &mut names.options,
        };
        *target = Some("invalid\0name".into());
        assert!(config.validate().is_err());
    }
    let config = KeyboardConfig {
        layout: Some("us,de".into()),
        variant: Some(",nodeadkeys".into()),
        options: Some(String::new()),
        ..Default::default()
    };
    config.validate().unwrap();
    assert_eq!(config.options.as_deref(), Some(""));
}

#[cfg(all(target_os = "linux", feature = "shell-wayland-linux"))]
#[test]
fn explicit_layout_changes_the_compiled_seat_keymap() {
    use crate::platform::linux::XkbKeyboard;
    let compile = |layout: &str| {
        let config = KeyboardConfig {
            include_root: None,
            rules: Some("evdev".into()),
            model: Some("pc105".into()),
            layout: Some(layout.into()),
            variant: Some(String::new()),
            options: Some(String::new()),
        };
        config.validate().unwrap();
        XkbKeyboard::from_names(
            config.rules.as_deref(),
            config.model.as_deref(),
            config.layout.as_deref(),
            config.variant.as_deref(),
            config.options.as_deref(),
        )
        .unwrap()
    };
    let us = compile("us");
    let de = compile("de");
    // The same physical evdev key is Y in US and Z in German QWERTZ.
    assert_eq!(us.utf8(21).unwrap(), "y");
    assert_eq!(de.utf8(21).unwrap(), "z");
    assert_ne!(us.keymap_string().unwrap(), de.keymap_string().unwrap());
}

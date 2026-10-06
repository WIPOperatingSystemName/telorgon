use super::{settings::*, *};

#[test]
fn ip_settings_roundtrip_addresses_routes_and_dns_with_correct_dbus_types() {
    for (family, address, gateway, dns, prefix) in [
        (IpFamily::V4, "192.0.2.10", "192.0.2.1", "1.1.1.1", 24),
        (
            IpFamily::V6,
            "2001:db8::10",
            "2001:db8::1",
            "2001:4860:4860::8888",
            64,
        ),
    ] {
        let address = IpAddress::new(address.parse().unwrap(), prefix).unwrap();
        let expected = IpSettings {
            method: IpMethod::Static(vec![address.clone()]),
            gateway: Some(gateway.parse().unwrap()),
            dns: vec![dns.parse().unwrap()],
            routes: vec![NetworkRoute {
                destination: address,
                gateway: Some(gateway.parse().unwrap()),
                metric: Some(20),
            }],
            ignore_automatic_dns: true,
            ignore_automatic_routes: true,
        };
        let mut values = Values::from([
            ("addresses".into(), array(vec![vec![1u32, 2, 3]]).unwrap()),
            ("custom-option".into(), string("preserved")),
        ]);
        encode_ip(&expected, family, &mut values).unwrap();
        assert!(!values.contains_key("addresses"));
        assert_eq!(text(&values, "custom-option").as_deref(), Some("preserved"));
        assert_eq!(decode_ip(Some(&values), family).unwrap(), expected);
        // Serialize through D-Bus to catch wrong variant signatures, including prefix widths.
        let bytes = zbus::zvariant::to_bytes(
            zbus::zvariant::serialized::Context::new_dbus(zbus::zvariant::LE, 0),
            &values,
        )
        .unwrap();
        let (decoded, _): (Values, usize) = bytes.deserialize().unwrap();
        assert_eq!(decode_ip(Some(&decoded), family).unwrap(), expected);
    }
}
#[test]
fn wifi_settings_preserve_raw_ssids_and_do_not_add_security_to_open_networks() {
    let ssid = Ssid::new(vec![0xff, b'A', 0]).unwrap();
    let options = WifiConnectOptions::new(ssid.clone(), WifiSecurity::Open);
    let settings = operations::wifi_settings(&options).unwrap();
    let bytes =
        Vec::<u8>::try_from(settings["802-11-wireless"]["ssid"].try_clone().unwrap()).unwrap();
    assert_eq!(bytes, ssid.as_bytes());
    assert!(!settings.contains_key("802-11-wireless-security"));
    let mut options = WifiConnectOptions::new(
        Ssid::new(b"Cafe".to_vec()).unwrap(),
        WifiSecurity::WpaPersonal,
    );
    assert!(matches!(
        operations::wifi_settings(&options),
        Err(NetworkError::CredentialsRequired)
    ));
    options.credentials = Some(NetworkSecret::new("test-password"));
    let settings = operations::wifi_settings(&options).unwrap();
    assert_eq!(
        text(&settings["802-11-wireless-security"], "key-mgmt").as_deref(),
        Some("wpa-psk")
    );
    assert!(format!("{options:?}").contains("[redacted]"));
    assert!(!format!("{options:?}").contains("test-password"));
}
#[test]
fn security_classification_does_not_mistake_encrypted_networks_for_open_wifi() {
    assert_eq!(snapshot::wifi_security(0, 0, 0), WifiSecurity::Open);
    assert_eq!(snapshot::wifi_security(1, 0, 0), WifiSecurity::Wep);
    assert_eq!(
        snapshot::wifi_security(1, 0, 0x100 | 0x400),
        WifiSecurity::WpaPersonal
    );
    assert_eq!(
        snapshot::wifi_security(1, 0, 0x400),
        WifiSecurity::Wpa3Personal
    );
    assert_eq!(
        snapshot::wifi_security(1, 0, 0x200),
        WifiSecurity::Enterprise
    );
    assert_eq!(snapshot::wifi_security(0, 0, 0x800), WifiSecurity::Unknown);
}
#[test]
fn replacing_backend_identity_invalidates_all_runtime_targets() {
    let mut provider = NetworkManagerProvider::default();
    let id = NetworkInterfaceId::new();
    provider.interfaces.insert("/device/1".into(), id);
    provider
        .profiles
        .insert("/profile/1".into(), NetworkProfileId::new());
    provider.reset_ids();
    provider
        .interfaces
        .insert("/device/1".into(), NetworkInterfaceId::new());
    assert_eq!(
        NetworkManagerProvider::path(&provider.interfaces, id),
        Err(NetworkError::Stale)
    );
    assert!(provider.profiles.is_empty());
}

#[test]
fn dhcp_failure_is_reported_separately_from_a_wifi_authentication_failure() {
    assert_eq!(
        snapshot::failure_reason(7),
        NetworkError::CredentialsRequired
    );
    assert_eq!(
        snapshot::failure_reason(10),
        NetworkError::AuthenticationFailed
    );
    assert_eq!(
        snapshot::failure_reason(5),
        NetworkError::IpConfigurationFailed
    );
    assert_eq!(
        snapshot::failure_reason(17),
        NetworkError::IpConfigurationFailed
    );
    assert_eq!(snapshot::failure_reason(999), NetworkError::BackendFailure);
}

#[test]
fn profile_interface_binding_preserves_names_and_treats_empty_names_as_unbound() {
    for name in ["", "eth0"] {
        let values = Settings::from([(
            "connection".into(),
            Values::from([
                ("type".into(), string("802-3-ethernet")),
                ("interface-name".into(), string(name)),
            ]),
        )]);
        let profile = profile(NetworkProfileId::new(), &values, true).unwrap();
        assert_eq!(
            profile.interface_name.as_deref(),
            if name.is_empty() { None } else { Some(name) }
        );
    }
}

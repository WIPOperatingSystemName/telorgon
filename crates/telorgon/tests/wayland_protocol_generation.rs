#![cfg(target_os = "linux")]

#[path = "../build/wayland.rs"]
mod generator;

use generator::schema::{self, parse};
use std::{ffi::CStr, ptr};
use telorgon::wayland_server::{ArgumentType, MessageKind, NativeProtocol};

#[test]
fn generated_descriptors_are_self_contained_and_have_static_lifetimes() {
    // This test deliberately uses no XML, environment variables, display, socket, or native server.
    let interface = NativeProtocol::desktop().interface("wl_surface").unwrap();
    let schema = NativeProtocol::desktop()
        .interface_schema("wl_surface")
        .unwrap();
    assert!(schema.version >= 6);
    assert_eq!(schema.request_named("attach").unwrap().0, 1);
    assert_eq!(
        schema.request_named("attach").unwrap().1.native_signature(),
        "?oii"
    );
    assert!(ptr::eq(
        interface,
        NativeProtocol::desktop().interface("wl_surface").unwrap()
    ));
    // Returning a 'static reference also compile-checks that temporary handles can disappear.
    let _: &'static telorgon::wayland_server::ffi::wl_interface = interface;
    for interface in NativeProtocol::desktop().schema().interfaces {
        let native = NativeProtocol::desktop().interface(interface.name).unwrap();
        assert_eq!(
            unsafe { CStr::from_ptr(native.name) }.to_str().unwrap(),
            interface.name
        );
        for (messages, pointer) in [
            (interface.requests, native.methods),
            (interface.events, native.events),
        ] {
            for (opcode, message) in messages.iter().enumerate() {
                let native = unsafe { &*pointer.add(opcode) };
                assert_eq!(
                    unsafe { CStr::from_ptr(native.signature) }
                        .to_str()
                        .unwrap(),
                    message.native_signature()
                );
                for (index, arg) in message.arguments.iter().enumerate() {
                    let pointer = unsafe { *native.types.add(index) };
                    let expected = arg
                        .interface
                        .and_then(|name| NativeProtocol::desktop().interface(name));
                    assert_eq!(pointer, expected.map_or(ptr::null(), |i| i as *const _));
                }
            }
        }
    }
}

#[test]
fn every_generated_field_matches_the_selected_xml() {
    let native = NativeProtocol::desktop();
    let mut count = 0;
    // Parse XML independently of the build model so this checks the generator, not just its output
    // against another use of the same signature builder. Preserve source opcode order.
    for (_, path) in generator::source_paths() {
        let xml = std::fs::read_to_string(path).unwrap();
        let doc = roxmltree::Document::parse(&xml).unwrap();
        for interface in doc
            .root_element()
            .children()
            .filter(|n| n.has_tag_name("interface"))
        {
            count += 1;
            let name = interface.attribute("name").unwrap();
            let metadata = native.interface_schema(name).unwrap();
            let raw = native.interface(name).unwrap();
            let version = interface
                .attribute("version")
                .unwrap()
                .parse::<u32>()
                .unwrap();
            assert_eq!(metadata.version, version);
            assert_eq!(raw.version, version as i32);
            for (tag, kind, messages, raw_count, raw_messages) in [
                (
                    "request",
                    MessageKind::Request,
                    metadata.requests,
                    raw.method_count,
                    raw.methods,
                ),
                (
                    "event",
                    MessageKind::Event,
                    metadata.events,
                    raw.event_count,
                    raw.events,
                ),
            ] {
                let nodes = interface
                    .children()
                    .filter(|n| n.has_tag_name(tag))
                    .collect::<Vec<_>>();
                assert_eq!(nodes.len(), messages.len());
                assert_eq!(raw_count as usize, nodes.len());
                for (opcode, (xml, message)) in nodes.iter().zip(messages).enumerate() {
                    assert_eq!(message.name, xml.attribute("name").unwrap());
                    assert_eq!(message.kind, kind);
                    assert_eq!(
                        message.since,
                        xml.attribute("since")
                            .unwrap_or("1")
                            .parse::<u32>()
                            .unwrap()
                    );
                    assert_eq!(
                        message.destructor,
                        xml.attribute("type") == Some("destructor")
                    );
                    let raw = unsafe { &*raw_messages.add(opcode) };
                    assert_eq!(
                        unsafe { CStr::from_ptr(raw.name) }.to_str().unwrap(),
                        message.name
                    );
                    let mut signature = if message.since > 1 {
                        message.since.to_string()
                    } else {
                        String::new()
                    };
                    let mut index = 0;
                    for arg in xml.children().filter(|n| n.has_tag_name("arg")) {
                        if arg.attribute("type") == Some("new_id")
                            && arg.attribute("interface").is_none()
                        {
                            signature.push_str("su");
                            assert_eq!(
                                message.arguments[index].argument_type,
                                ArgumentType::String
                            );
                            assert_eq!(
                                message.arguments[index + 1].argument_type,
                                ArgumentType::Uint
                            );
                            index += 2;
                        }
                        let generated = &message.arguments[index];
                        assert_eq!(generated.name, arg.attribute("name").unwrap());
                        assert_eq!(generated.interface, arg.attribute("interface"));
                        assert_eq!(
                            generated.allow_null,
                            arg.attribute("allow-null") == Some("true")
                        );
                        let (ty, symbol) = match arg.attribute("type").unwrap() {
                            "int" => (ArgumentType::Int, 'i'),
                            "uint" => (ArgumentType::Uint, 'u'),
                            "fixed" => (ArgumentType::Fixed, 'f'),
                            "string" => (ArgumentType::String, 's'),
                            "object" => (ArgumentType::Object, 'o'),
                            "new_id" => (ArgumentType::NewId, 'n'),
                            "array" => (ArgumentType::Array, 'a'),
                            "fd" => (ArgumentType::Fd, 'h'),
                            _ => unreachable!(),
                        };
                        assert_eq!(generated.argument_type, ty);
                        if generated.allow_null {
                            signature.push('?');
                        }
                        signature.push(symbol);
                        index += 1;
                    }
                    assert_eq!(index, message.arguments.len());
                    assert_eq!(signature, message.native_signature());
                }
            }
        }
    }
    assert_eq!(count, native.schema().interfaces.len());
}

fn fixture(contents: &str) -> String {
    format!(
        "<protocol name='test'><interface name='test' version='2'>{contents}</interface></protocol>"
    )
}

#[test]
fn malformed_and_incompatible_schemas_are_rejected() {
    for xml in [
        "<protocol name='test'>",
        "<protocol name='test'></interface></protocol>",
        "<protocol name='test'/><protocol name='other'/>",
        "<!DOCTYPE protocol [<!ENTITY x 'test'>]><protocol name='&x;'/>",
        "<protocol name='test'><interface name='bad&#0;' version='1'/></protocol>",
        "<protocol name='test'><interface name='test' version='2147483648'/></protocol>",
        "<protocol name='test'><interface name='test' version='0'/></protocol>",
        "<protocol name='test'><interface name='test' version='1'/><interface name='test' version='1'/></protocol>",
        "<protocol name='test'><interface name='test'/></protocol>",
    ] {
        assert!(parse(xml).is_err(), "accepted {xml}");
    }
    for content in [
        "<request name='bad' since='3'/>",
        "<request name='bad' since='0'/>",
        "<request name='bad' type='other'/>",
        "<request name='bad'><arg name='x' type='other'/></request>",
        "<request name='bad'><arg name='x' type='uint' allow-null='true'/></request>",
        "<request name='bad'><arg name='x' type='object' allow-null='yes'/></request>",
        "<request name='bad'><arg name='x' type='fd' interface='test'/></request>",
        "<event name='bad'><arg name='id' type='new_id'/></event>",
        "<request name='a'/><request name='a'/>",
        "<request name='bad'><arg name='x' type='array' allow-null='true'/></request>",
        "<request name='bad'><arg name='x' type='new_id' interface='test'/><arg name='y' type='new_id' interface='test'/></request>",
        "<request name='bad'><arg name='x' type='int'/><arg name='x' type='int'/></request>",
        "<description><request name='hidden'/></description>",
        "<arg name='x' type='int'/>",
    ] {
        assert!(parse(&fixture(content)).is_err(), "accepted {content}");
    }
    assert!(parse(&" ".repeat(schema::MAX_SOURCE_BYTES + 1)).is_err());
    let args = (0..21)
        .map(|i| format!("<arg name='a{i}' type='uint'/>"))
        .collect::<String>();
    assert!(parse(&fixture(&format!("<request name='large'>{args}</request>"))).is_err());
    let messages = (0..4097)
        .map(|i| format!("<request name='r{i}'/>"))
        .collect::<String>();
    assert!(parse(&fixture(&messages)).is_err());
    let interfaces = (0..1025)
        .map(|i| format!("<interface name='i{i}' version='1'/>"))
        .collect::<String>();
    assert!(parse(&format!("<protocol name='large'>{interfaces}</protocol>")).is_err());
}

#[test]
fn profile_duplicate_and_reference_validation() {
    let profile = *generator::profile::DESKTOP_PROTOCOLS
        .iter()
        .find(|profile| profile.name == "wayland")
        .expect("core Wayland profile");
    let mut protocol = parse(&fixture("")).unwrap();
    assert!(generator::validate_profile(&profile, &protocol).is_err());
    protocol.name = "wayland".into();
    assert!(
        generator::validate_profile(&profile, &protocol)
            .unwrap_err()
            .contains("missing interface")
    );
    let mut catalog = generator::load_catalog().unwrap();
    let surface = catalog.iter_mut().find(|i| i.name == "wl_surface").unwrap();
    surface.version = 5;
    protocol.interfaces = catalog.clone();
    assert!(
        generator::validate_profile(&profile, &protocol)
            .unwrap_err()
            .contains("profile requires 6")
    );
    catalog.push(catalog[0].clone());
    assert!(
        generator::validate_interfaces(&catalog)
            .unwrap_err()
            .contains("duplicate interface")
    );
    let unknown = parse(&fixture(
        "<request name='test'><arg name='object' type='object' interface='missing'/></request>",
    ))
    .unwrap();
    assert!(
        generator::validate_interfaces(&unknown.interfaces)
            .unwrap_err()
            .contains("unresolved interface missing")
    );
}

#[test]
fn pinned_wire_contract_rejects_dispatch_breaking_changes_but_accepts_future_versions() {
    let catalog = generator::load_catalog().unwrap();
    let contract = include_str!("../build/protocol-wire-contract.txt");
    for mutation in 0..6 {
        let mut changed = catalog.clone();
        let interface = changed.iter_mut().find(|i| i.name == "wl_surface").unwrap();
        match mutation {
            0 => interface.requests.swap(0, 1),
            1 => interface.requests[0].destructor = false,
            2 => interface.requests[1].arguments[0].allow_null = false,
            3 => interface.requests[1].arguments[0].interface = Some("wl_surface".into()),
            4 => interface.requests[1].since = 2,
            _ => interface.requests[1].arguments[1].kind = "Uint",
        }
        assert!(generator::validate_contract(&changed, contract).is_err());
    }
    let mut future = catalog.clone();
    let surface = future.iter_mut().find(|i| i.name == "wl_surface").unwrap();
    surface.version += 1;
    let mut request = surface.requests[0].clone();
    request.name = "future".into();
    request.since = surface.version;
    surface.requests.push(request);
    generator::validate_contract(&future, contract).unwrap();
}

#[test]
fn generic_new_id_expands_signature_and_wire_slots() {
    let protocol = NativeProtocol::desktop();
    let bind = &protocol.interface_schema("wl_registry").unwrap().requests[0];
    assert_eq!(bind.native_signature(), "usun");
    assert_eq!(
        bind.arguments
            .iter()
            .map(|a| a.argument_type)
            .collect::<Vec<_>>(),
        [
            ArgumentType::Uint,
            ArgumentType::String,
            ArgumentType::Uint,
            ArgumentType::NewId
        ]
    );
}

#[test]
#[ignore = "requires the official wayland-scanner executable; run explicitly for reference audit"]
fn signatures_match_official_c_scanner() {
    use std::process::Command;
    let protocol = NativeProtocol::desktop();
    let mut checked = 0;
    for (_, path) in generator::source_paths() {
        let result = Command::new("wayland-scanner")
            .arg("private-code")
            .arg(path)
            .arg("/dev/stdout")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let c = String::from_utf8(result.stdout).unwrap();
        for interface in protocol.schema().interfaces {
            for (kind, messages) in [
                ("requests", interface.requests),
                ("events", interface.events),
            ] {
                let marker = format!(
                    "static const struct wl_message {}_{kind}[] = {{",
                    interface.name
                );
                let Some((_, tail)) = c.split_once(&marker) else {
                    continue;
                };
                let block = tail.split_once("};").unwrap().0;
                let rows = block
                    .lines()
                    .filter(|line| line.contains('{'))
                    .collect::<Vec<_>>();
                assert_eq!(rows.len(), messages.len());
                checked += rows.len();
                for (row, message) in rows.iter().zip(messages) {
                    assert!(
                        row.contains(&format!(
                            "\"{}\", \"{}\"",
                            message.name,
                            message.native_signature()
                        )),
                        "{row}"
                    );
                }
            }
        }
    }
    assert_eq!(
        checked,
        protocol
            .schema()
            .interfaces
            .iter()
            .map(|i| i.requests.len() + i.events.len())
            .sum::<usize>()
    );
}

#[test]
fn generated_request_metadata_preserves_typed_argument_decoding() {
    use telorgon::wayland_server::{IncomingRequest, ffi::wl_argument};
    let protocol = NativeProtocol::desktop();
    let attach = protocol
        .interface_schema("wl_surface")
        .unwrap()
        .request_named("attach")
        .unwrap()
        .1;
    let mut args = [
        wl_argument { o: ptr::null_mut() },
        wl_argument { i: -3 },
        wl_argument { i: 7 },
    ];
    let request = unsafe { IncomingRequest::from_raw(attach, args.as_mut_ptr()) }.unwrap();
    assert!(request.object(0).unwrap().is_none());
    assert_eq!(request.int(1).unwrap(), -3);
    assert_eq!(request.int(2).unwrap(), 7);
    assert!(request.uint(1).is_err());
    assert!(request.int(3).is_err());
}

#[test]
fn missing_protocol_file_reports_source_and_build_overrides() {
    let path = std::env::temp_dir()
        .join(format!("telorgon-missing-{}", std::process::id()))
        .join("wayland.xml");
    assert!(!path.exists());
    let error =
        generator::load_protocol(&generator::profile::DESKTOP_PROTOCOLS[0], &path).unwrap_err();
    assert!(error.contains("wayland"));
    assert!(error.contains(path.to_str().unwrap()));
    assert!(error.contains(generator::WAYLAND_XML_ENV));
    assert!(error.contains(generator::PROTOCOLS_ENV));
}

#[test]
fn wire_argument_limit_counts_generic_new_id_expansion() {
    let args = (0..17)
        .map(|i| format!("<arg name='a{i}' type='uint'/>"))
        .collect::<String>();
    let request = format!("<request name='bind'>{args}<arg name='id' type='new_id'/></request>");
    assert_eq!(
        parse(&fixture(&request)).unwrap().interfaces[0].requests[0]
            .arguments
            .len(),
        20
    );
    let too_large = request.replace("</request>", "<arg name='extra' type='uint'/></request>");
    assert!(
        parse(&fixture(&too_large))
            .unwrap_err()
            .contains("libwayland limit of 20")
    );
}

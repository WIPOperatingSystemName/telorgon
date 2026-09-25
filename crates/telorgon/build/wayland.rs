//! Build-time catalog, compatibility checks, and Rust descriptor emission.
#![allow(dead_code)] // Also compiled by integration tests, which exercise individual stages.

use std::{
    collections::BTreeMap,
    env,
    fmt::Write,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[path = "../src/integrations/wayland/server/protocol.rs"]
pub mod profile;
pub mod schema;
use schema::{Interface, Protocol};

pub const WAYLAND_XML_ENV: &str = "TELORGON_WAYLAND_XML";
pub const PROTOCOLS_ENV: &str = "TELORGON_WAYLAND_PROTOCOLS_DIR";

pub fn source_paths() -> Vec<(&'static profile::ProtocolSpec, PathBuf)> {
    let core = env::var_os(WAYLAND_XML_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/share/wayland/wayland.xml".into());
    let extensions = env::var_os(PROTOCOLS_ENV)
        .map(PathBuf::from)
        .unwrap_or_else(|| "/usr/share/wayland-protocols".into());
    profile::DESKTOP_PROTOCOLS
        .iter()
        .map(|profile| {
            let path = if profile.name == "wayland" {
                core.clone()
            } else if profile.name == "server-decoration" {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../")
                    .join(profile.source)
            } else {
                extensions.join(profile.source)
            };
            (profile, path)
        })
        .collect()
}

pub fn validate_profile(
    profile: &profile::ProtocolSpec,
    protocol: &Protocol,
) -> Result<(), String> {
    let xml_name = match profile.name {
        "linux-explicit-synchronization-unstable-v1" => {
            "zwp_linux_explicit_synchronization_unstable_v1".to_owned()
        }
        name => name.replace('-', "_"),
    };
    if protocol.name != xml_name {
        return Err(format!(
            "expected protocol {}, found {}",
            profile.name, protocol.name
        ));
    }
    for expected in profile.interfaces {
        let interface = protocol
            .interfaces
            .iter()
            .find(|interface| interface.name == expected.name)
            .ok_or_else(|| format!("missing interface {}", expected.name))?;
        if interface.version < expected.source_version {
            return Err(format!(
                "interface {} has source version {}, profile requires {}",
                expected.name, interface.version, expected.source_version
            ));
        }
    }
    Ok(())
}

pub fn load_protocol(profile: &profile::ProtocolSpec, path: &Path) -> Result<Protocol, String> {
    let load = || -> Result<Protocol, String> {
        let mut source = String::new();
        fs::File::open(path)
            .map_err(|e| e.to_string())?
            .take(schema::MAX_SOURCE_BYTES as u64 + 1)
            .read_to_string(&mut source)
            .map_err(|e| e.to_string())?;
        let protocol = schema::parse(&source)?;
        validate_profile(profile, &protocol)?;
        Ok(protocol)
    };
    load().map_err(|error| format!("{} at {}: {error}. Install compatible Wayland/wayland-protocols development data or set {WAYLAND_XML_ENV} and {PROTOCOLS_ENV} for the build", profile.name, path.display()))
}

pub fn load_catalog() -> Result<Vec<Interface>, String> {
    let mut interfaces = Vec::new();
    for (profile, path) in source_paths() {
        interfaces.extend(load_protocol(profile, &path)?.interfaces);
    }
    validate_interfaces(&interfaces)?;
    validate_contract(&interfaces, include_str!("protocol-wire-contract.txt"))?;
    Ok(interfaces)
}

// The existing cursor-shape profile references this unimplemented tablet interface. Its request
// is rejected by the compositor; keep that one argument opaque, as before. All other unresolved
// references are build errors. This exception does not advertise tablet support.
pub fn opaque_reference(interface: &str, message: &str, argument: &schema::Argument) -> bool {
    interface == "wp_cursor_shape_manager_v1"
        && message == "get_tablet_tool_v2"
        && argument.name == "tablet_tool"
        && argument.kind == "Object"
        && argument.interface.as_deref() == Some("zwp_tablet_tool_v2")
}

pub fn validate_interfaces(interfaces: &[Interface]) -> Result<(), String> {
    let mut names = BTreeMap::new();
    for (index, interface) in interfaces.iter().enumerate() {
        if names.insert(interface.name.as_str(), index).is_some() {
            return Err(format!(
                "duplicate interface {} across protocol sources",
                interface.name
            ));
        }
    }
    for interface in interfaces {
        for message in interface.requests.iter().chain(&interface.events) {
            for arg in &message.arguments {
                if let Some(name) = &arg.interface {
                    if !names.contains_key(name.as_str())
                        && !opaque_reference(&interface.name, &message.name, arg)
                    {
                        return Err(format!(
                            "{}.{}, argument {}: unresolved interface {name}",
                            interface.name, message.name, arg.name
                        ));
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn contract(interfaces: &[Interface]) -> String {
    let mut lines = Vec::new();
    for interface in interfaces {
        let Some(expected) = profile::interface(&interface.name) else {
            continue;
        };
        for (kind, messages) in [
            ("request", &interface.requests),
            ("event", &interface.events),
        ] {
            for (opcode, message) in messages.iter().enumerate() {
                if message.since <= expected.source_version {
                    lines.push(message.contract(&interface.name, kind, opcode));
                }
            }
        }
    }
    lines.sort();
    lines.join("\n")
}

pub fn validate_contract(interfaces: &[Interface], expected: &str) -> Result<(), String> {
    let actual = contract(interfaces);
    let expected = expected
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    if actual != expected {
        let actual_lines = actual.lines().collect::<Vec<_>>();
        let expected_lines = expected.lines().collect::<Vec<_>>();
        let missing = expected_lines
            .iter()
            .find(|line| !actual_lines.contains(line));
        let changed = actual_lines
            .iter()
            .find(|line| !expected_lines.contains(line));
        return Err(format!(
            "incompatible protocol wire contract (opcode/name/signature/destructor/object types): expected {missing:?}; unexpected {changed:?}. Check the selected XML files; the pinned profile ABI must remain compatible"
        ));
    }
    Ok(())
}

pub fn emit(interfaces: &[Interface]) -> String {
    let indices = interfaces
        .iter()
        .enumerate()
        .map(|(i, interface)| (interface.name.as_str(), i))
        .collect::<BTreeMap<_, _>>();
    let mut output = String::from(
        "// Generated by build/wayland.rs. Do not edit.\nuse crate::wayland_server::{ArgumentSchema, ArgumentType, MessageSchema, MessageKind};\n",
    );
    let mut native = String::new();
    let mut schemas = String::new();
    for (i, interface) in interfaces.iter().enumerate() {
        let mut schema_messages = Vec::new();
        let mut native_messages = Vec::new();
        for (kind, messages) in [
            ("Request", &interface.requests),
            ("Event", &interface.events),
        ] {
            let prefix = format!("I{i}_{kind}").to_uppercase();
            let mut values = String::new();
            let mut metadata = String::new();
            for (opcode, message) in messages.iter().enumerate() {
                let types = format!("{prefix}_{opcode}_TYPES");
                write!(
                    output,
                    "static {types}: Types<{}> = Types([",
                    message.arguments.len()
                )
                .unwrap();
                let mut args = String::new();
                for arg in &message.arguments {
                    match arg.interface.as_deref().and_then(|name| indices.get(name)) {
                        Some(index) => {
                            write!(output, "&INTERFACES.0[{index}] as *const wl_interface,")
                                .unwrap()
                        }
                        None => output.push_str("std::ptr::null(),"),
                    }
                    let target = arg
                        .interface
                        .as_ref()
                        .map_or("None".into(), |name| format!("Some({name:?})"));
                    write!(args, "ArgumentSchema {{ name: {:?}, argument_type: ArgumentType::{}, interface: {target}, allow_null: {} }},", arg.name, arg.kind, arg.allow_null).unwrap();
                }
                output.push_str("]);\n");
                let signature = message.signature();
                write!(values, "wl_message {{ name: c{:?}.as_ptr(), signature: c{signature:?}.as_ptr(), types: {types}.0.as_ptr() }},", message.name).unwrap();
                write!(metadata, "MessageSchema {{ name: {:?}, since: {}, signature: {signature:?}, destructor: {}, kind: MessageKind::{kind}, arguments: &[{args}] }},", message.name, message.since, message.destructor).unwrap();
            }
            if !messages.is_empty() {
                writeln!(
                    output,
                    "static {prefix}: Messages<{}> = Messages([{values}]);",
                    messages.len()
                )
                .unwrap();
            }
            schema_messages.push(format!("&[{metadata}]"));
            native_messages.push(if messages.is_empty() {
                "std::ptr::null()".into()
            } else {
                format!("{prefix}.0.as_ptr()")
            });
        }
        writeln!(native, "wl_interface {{ name: c{:?}.as_ptr(), version: {}, method_count: {}, methods: {}, event_count: {}, events: {} }},", interface.name, interface.version, interface.requests.len(), native_messages[0], interface.events.len(), native_messages[1]).unwrap();
        writeln!(
            schemas,
            "InterfaceSchema {{ name: {:?}, version: {}, requests: {}, events: {} }},",
            interface.name, interface.version, schema_messages[0], schema_messages[1]
        )
        .unwrap();
    }
    writeln!(
        output,
        "static INTERFACES: Interfaces<{}> = Interfaces([{native}]);",
        interfaces.len()
    )
    .unwrap();
    writeln!(output, "static SCHEMA: ProtocolSchema = ProtocolSchema {{ name: \"telorgon-desktop\", interfaces: &[{schemas}] }};").unwrap();
    output
}

pub fn generate() -> Result<(), String> {
    println!("cargo:rerun-if-env-changed={WAYLAND_XML_ENV}");
    println!("cargo:rerun-if-env-changed={PROTOCOLS_ENV}");
    for (_, path) in source_paths() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    let interfaces = load_catalog()?;
    let output = PathBuf::from(env::var_os("OUT_DIR").ok_or("Cargo did not set OUT_DIR")?)
        .join("wayland_descriptors.rs");
    fs::write(output, emit(&interfaces)).map_err(|e| e.to_string())
}

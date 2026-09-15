//! Procedural authoring helpers for Telorgon's composition and asset APIs.

use proc_macro::TokenStream;
use proc_macro_crate::{FoundCrate, crate_name};
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component as PathComponent, Path as FilePath, PathBuf};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    Attribute, Fields, Ident, ItemStruct, LitStr, Meta, Path, Token, Visibility, parse_macro_input,
};

struct AssetCatalogInput {
    visibility: Visibility,
    name: Ident,
    root: LitStr,
}

impl syn::parse::Parse for AssetCatalogInput {
    fn parse(input: syn::parse::ParseStream<'_>) -> syn::Result<Self> {
        let visibility = input.parse()?;
        input.parse::<Token![mod]>()?;
        let name = input.parse()?;
        input.parse::<Token![=]>()?;
        let root = input.parse()?;
        let _ = input.parse::<Token![;]>();
        if !input.is_empty() {
            return Err(input.error("expected `asset_catalog! { pub mod name = \"assets\"; }`"));
        }
        Ok(Self {
            visibility,
            name,
            root,
        })
    }
}

#[derive(Clone)]
struct CatalogFile {
    modules: Vec<String>,
    identifier: String,
    key: String,
    absolute: String,
    kind: CatalogKind,
    media_type: &'static str,
}

#[derive(Clone, Copy)]
enum CatalogKind {
    Icon,
    Image,
    Cursor,
    CursorTheme,
}

#[derive(Default)]
struct CatalogModule {
    modules: BTreeMap<String, CatalogModule>,
    files: Vec<CatalogFile>,
}

/// Embeds a project asset directory and generates nested, typed constants plus `bundle()`.
///
/// Directory names become modules. Files below `icons/`, `images/`, and `cursors/` become
/// `IconAsset`, `ImageAsset`, and `CursorAsset` constants. TOML files below `cursors/` become
/// `CursorThemeAsset` constants.
///
/// ```ignore
/// telorgon::asset_catalog! { pub mod assets = "assets"; }
/// let icon = assets::icons::APP;
/// let bundle = assets::bundle();
/// ```
#[proc_macro]
pub fn asset_catalog(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as AssetCatalogInput);
    match expand_asset_catalog(input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}

fn expand_asset_catalog(input: AssetCatalogInput) -> syn::Result<proc_macro2::TokenStream> {
    let declared_root = PathBuf::from(input.root.value());
    if declared_root.is_absolute()
        || declared_root.components().any(|component| {
            matches!(
                component,
                PathComponent::ParentDir | PathComponent::RootDir | PathComponent::Prefix(_)
            )
        })
    {
        return Err(syn::Error::new_spanned(
            input.root,
            "asset catalog root must be a project-relative directory without `..`",
        ));
    }
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")
        .map(PathBuf::from)
        .ok_or_else(|| syn::Error::new_spanned(&input.root, "CARGO_MANIFEST_DIR is unavailable"))?;
    let absolute_root = manifest.join(&declared_root);
    if !absolute_root.is_dir() {
        return Err(syn::Error::new_spanned(
            &input.root,
            format!(
                "asset catalog directory `{}` does not exist",
                absolute_root.display()
            ),
        ));
    }

    let mut files = Vec::new();
    collect_catalog_files(&absolute_root, &absolute_root, &mut files).map_err(|message| {
        syn::Error::new_spanned(
            &input.root,
            format!("could not build asset catalog: {message}"),
        )
    })?;
    files.sort_by(|left, right| left.key.cmp(&right.key));
    let tree =
        catalog_tree(&files).map_err(|message| syn::Error::new_spanned(&input.root, message))?;
    let crate_path = telorgon_crate_path()?;
    let nested = render_catalog_module(&tree, &crate_path);
    let entries = files.iter().map(|file| {
        let key = LitStr::new(&file.key, proc_macro2::Span::call_site());
        let absolute = LitStr::new(&file.absolute, proc_macro2::Span::call_site());
        let media_type = LitStr::new(file.media_type, proc_macro2::Span::call_site());
        let kind = kind_tokens(file.kind, &crate_path);
        quote! {
            #crate_path::AssetEntry::embedded(
                #crate_path::AssetKey::new(#key),
                #kind,
                #media_type,
                include_bytes!(#absolute),
            )
        }
    });
    let entry_count = files.len();
    let visibility = input.visibility;
    let name = input.name;
    Ok(quote! {
        #visibility mod #name {
            #nested

            #[doc(hidden)]
            static __TELORGON_ASSETS: [#crate_path::AssetEntry; #entry_count] = [#(#entries),*];

            /// Marker implementing `AssetCatalog` for generic registration helpers.
            pub struct Catalog;

            impl #crate_path::AssetCatalog for Catalog {
                const BUNDLE: #crate_path::AssetBundle =
                    #crate_path::AssetBundle::new(&__TELORGON_ASSETS);
            }

            /// Returns the immutable bundle shared by GUI and shell-environment runtimes.
            pub const fn bundle() -> #crate_path::AssetBundle {
                <Catalog as #crate_path::AssetCatalog>::BUNDLE
            }
        }
    })
}

fn telorgon_crate_path() -> syn::Result<proc_macro2::TokenStream> {
    match crate_name("telorgon") {
        Ok(FoundCrate::Itself) => Ok(quote!(::telorgon)),
        Ok(FoundCrate::Name(name)) => {
            let name = format_ident!("{name}");
            Ok(quote!(::#name))
        }
        Err(error) => Err(syn::Error::new(
            proc_macro2::Span::call_site(),
            format!("asset_catalog! must be used through a telorgon dependency: {error}"),
        )),
    }
}

fn collect_catalog_files(
    root: &FilePath,
    directory: &FilePath,
    output: &mut Vec<CatalogFile>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("cannot read `{}`: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("cannot enumerate `{}`: {error}", directory.display()))?;
    entries.sort_by_key(fs::DirEntry::file_name);
    for entry in entries {
        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if file_name.starts_with('.') {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect `{}`: {error}", entry.path().display()))?;
        if file_type.is_symlink() {
            return Err(format!(
                "symlinked asset `{}` is not allowed",
                entry.path().display()
            ));
        }
        if file_type.is_dir() {
            collect_catalog_files(root, &entry.path(), output)?;
            continue;
        }
        if !file_type.is_file() {
            continue;
        }
        let metadata = entry
            .metadata()
            .map_err(|error| format!("cannot inspect `{}`: {error}", entry.path().display()))?;
        if metadata.len() > 16 * 1024 * 1024 {
            return Err(format!(
                "asset `{}` exceeds the 16 MiB embedded-file limit",
                entry.path().display()
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| "asset escaped its catalog root".to_owned())?
            .to_path_buf();
        let components = relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        let (file, directories) = components
            .split_last()
            .ok_or_else(|| "empty asset path".to_owned())?;
        let extension = FilePath::new(file)
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let kind = classify_asset(directories.first().map(String::as_str), &extension)
            .ok_or_else(|| format!("unsupported asset type `{}`", relative.display()))?;
        let media_type = media_type(&extension)
            .ok_or_else(|| format!("unsupported asset type `{}`", relative.display()))?;
        let stem = FilePath::new(file)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .ok_or_else(|| format!("asset name `{file}` is not valid UTF-8"))?;
        output.push(CatalogFile {
            modules: directories
                .iter()
                .map(|directory| rust_identifier(directory, false))
                .collect::<Result<Vec<_>, _>>()?,
            identifier: rust_identifier(stem, true)?,
            key: components.join("/"),
            absolute: entry.path().to_string_lossy().into_owned(),
            kind,
            media_type,
        });
    }
    Ok(())
}

fn classify_asset(top: Option<&str>, extension: &str) -> Option<CatalogKind> {
    let top = top.unwrap_or_default().to_ascii_lowercase();
    match top.as_str() {
        "icons" => matches!(extension, "svg" | "png" | "jpg" | "jpeg" | "webp" | "ico")
            .then_some(CatalogKind::Icon),
        "cursors" if extension == "toml" => Some(CatalogKind::CursorTheme),
        "cursors" => matches!(extension, "svg" | "png" | "webp").then_some(CatalogKind::Cursor),
        "images" => matches!(
            extension,
            "svg" | "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
        )
        .then_some(CatalogKind::Image),
        _ => None,
    }
}

fn media_type(extension: &str) -> Option<&'static str> {
    match extension {
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "ico" => Some("image/x-icon"),
        "toml" => Some("application/toml"),
        _ => None,
    }
}

fn rust_identifier(value: &str, constant: bool) -> Result<String, String> {
    let mut result = String::new();
    for (index, character) in value.chars().enumerate() {
        let valid = character.is_ascii_alphanumeric() || character == '_';
        let character = if valid { character } else { '_' };
        if index == 0 && character.is_ascii_digit() {
            result.push('_');
        }
        result.push(if constant {
            character.to_ascii_uppercase()
        } else {
            character.to_ascii_lowercase()
        });
    }
    if result.is_empty() {
        return Err(format!("`{value}` cannot become a Rust identifier"));
    }
    Ok(result)
}

fn catalog_tree(files: &[CatalogFile]) -> Result<CatalogModule, String> {
    let mut root = CatalogModule::default();
    for file in files {
        let mut module = &mut root;
        for part in &file.modules {
            module = module.modules.entry(part.clone()).or_default();
        }
        module.files.push(file.clone());
    }
    validate_catalog_names(&root, "catalog")?;
    Ok(root)
}

fn validate_catalog_names(module: &CatalogModule, path: &str) -> Result<(), String> {
    let mut names = BTreeSet::new();
    for name in module.modules.keys() {
        if !names.insert(name.clone()) {
            return Err(format!("duplicate generated module `{path}::{name}`"));
        }
    }
    for file in &module.files {
        if !names.insert(file.identifier.clone()) {
            return Err(format!(
                "asset names collide after Rust identifier normalization at `{path}::{}`",
                file.identifier
            ));
        }
    }
    for (name, child) in &module.modules {
        validate_catalog_names(child, &format!("{path}::{name}"))?;
    }
    Ok(())
}

fn render_catalog_module(
    module: &CatalogModule,
    crate_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    let constants = module.files.iter().map(|file| {
        let identifier = format_ident!("{}", file.identifier);
        let key = LitStr::new(&file.key, proc_macro2::Span::call_site());
        let (asset_type, _) = asset_tokens(file.kind, crate_path);
        quote! {
            pub const #identifier: #asset_type =
                #asset_type::new(#crate_path::AssetKey::new(#key));
        }
    });
    let modules = module.modules.iter().map(|(name, child)| {
        let identifier = format_ident!("{name}");
        let contents = render_catalog_module(child, crate_path);
        quote! { pub mod #identifier { #contents } }
    });
    quote! { #(#constants)* #(#modules)* }
}

fn asset_tokens(
    kind: CatalogKind,
    crate_path: &proc_macro2::TokenStream,
) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    match kind {
        CatalogKind::Icon => (
            quote!(#crate_path::IconAsset),
            quote!(#crate_path::AssetKind::Icon),
        ),
        CatalogKind::Image => (
            quote!(#crate_path::ImageAsset),
            quote!(#crate_path::AssetKind::Image),
        ),
        CatalogKind::Cursor => (
            quote!(#crate_path::CursorAsset),
            quote!(#crate_path::AssetKind::Cursor),
        ),
        CatalogKind::CursorTheme => (
            quote!(#crate_path::CursorThemeAsset),
            quote!(#crate_path::AssetKind::CursorTheme),
        ),
    }
}

fn kind_tokens(
    kind: CatalogKind,
    crate_path: &proc_macro2::TokenStream,
) -> proc_macro2::TokenStream {
    asset_tokens(kind, crate_path).1
}

enum InputMode {
    Compare,
    Always,
    CompareWith(Path),
}

enum FieldKind {
    Input(InputMode),
    State,
}

/// Classifies a persistent component's fields as parent-owned inputs or locally persistent state.
#[proc_macro_attribute]
pub fn component(arguments: TokenStream, item: TokenStream) -> TokenStream {
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let arguments = match parser.parse(arguments) {
        Ok(arguments) => arguments,
        Err(error) => return error.into_compile_error().into(),
    };
    let mut crate_path = None;
    let mut derive_default = true;
    for argument in arguments {
        match argument {
            Meta::Path(path) if path.is_ident("no_default") => {
                derive_default = false;
            }
            Meta::NameValue(value) if value.path.is_ident("crate_path") => {
                if crate_path.is_some() {
                    return syn::Error::new_spanned(value, "duplicate crate_path argument")
                        .into_compile_error()
                        .into();
                }
                let syn::Expr::Path(path) = value.value else {
                    return syn::Error::new_spanned(value.value, "crate_path must be a Rust path")
                        .into_compile_error()
                        .into();
                };
                crate_path = Some(path.path);
            }
            other => {
                return syn::Error::new_spanned(other, "expected no_default or crate_path = path")
                    .into_compile_error()
                    .into();
            }
        }
    }

    let fields_trait = if let Some(path) = crate_path {
        quote!(#path::__private::ComponentFields)
    } else {
        match crate_name("telorgon") {
            Ok(FoundCrate::Itself) => quote!(::telorgon::compose::__private::ComponentFields),
            Ok(FoundCrate::Name(name)) => {
                let name = format_ident!("{name}");
                quote!(::#name::compose::__private::ComponentFields)
            }
            Err(error) => {
                return syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("#[component] must be used through a telorgon dependency: {error}"),
                )
                .into_compile_error()
                .into();
            }
        }
    };

    let mut item = parse_macro_input!(item as ItemStruct);
    let Fields::Named(fields) = &mut item.fields else {
        return syn::Error::new_spanned(&item, "#[component] requires a struct with named fields")
            .into_compile_error()
            .into();
    };

    let mut classified = Vec::with_capacity(fields.named.len());
    for field in &mut fields.named {
        let mut kind = None;
        let mut retained = Vec::with_capacity(field.attrs.len());
        for attribute in std::mem::take(&mut field.attrs) {
            if attribute.path().is_ident("input") {
                if kind.is_some() {
                    return syn::Error::new_spanned(
                        attribute,
                        "a component field must have exactly one of #[input] or #[state]",
                    )
                    .into_compile_error()
                    .into();
                }
                match parse_input_mode(&attribute) {
                    Ok(mode) => kind = Some(FieldKind::Input(mode)),
                    Err(error) => return error.into_compile_error().into(),
                }
            } else if attribute.path().is_ident("state") {
                if kind.is_some() {
                    return syn::Error::new_spanned(
                        attribute,
                        "a component field must have exactly one of #[input] or #[state]",
                    )
                    .into_compile_error()
                    .into();
                }
                if !matches!(attribute.meta, Meta::Path(_)) {
                    return syn::Error::new_spanned(
                        attribute,
                        "#[state] does not accept arguments",
                    )
                    .into_compile_error()
                    .into();
                }
                kind = Some(FieldKind::State);
            } else {
                retained.push(attribute);
            }
        }
        field.attrs = retained;
        let Some(kind) = kind else {
            return syn::Error::new_spanned(
                field,
                "every #[component] field must be marked #[input] or #[state]",
            )
            .into_compile_error()
            .into();
        };
        classified.push(kind);
    }

    if derive_default && !has_default_derive(&item.attrs) {
        item.attrs.push(syn::parse_quote!(#[derive(Default)]));
    }

    let fields: Vec<_> = fields.named.iter().cloned().collect();
    let name = &item.ident;
    let generics = &item.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let identifiers: Vec<_> = fields
        .iter()
        .map(|field| field.ident.as_ref().expect("named field"))
        .collect();
    let incoming_identifiers: Vec<_> = identifiers
        .iter()
        .map(|identifier| format_ident!("__telorgon_incoming_{identifier}"))
        .collect();

    let destructure = identifiers
        .iter()
        .zip(incoming_identifiers.iter())
        .map(|(field, incoming)| quote!(#field: #incoming));

    let input_fields: Vec<_> = fields
        .iter()
        .zip(classified.iter())
        .filter_map(|(field, kind)| match kind {
            FieldKind::Input(mode) => Some((field, mode)),
            FieldKind::State => None,
        })
        .collect();

    let snapshot_types = input_fields.iter().map(|(field, _)| &field.ty);
    let snapshot_values = input_fields.iter().map(|(field, _)| {
        let identifier = field.ident.as_ref().expect("named field");
        quote!(self.#identifier.clone())
    });
    let snapshot_bindings: Vec<_> = input_fields
        .iter()
        .map(|(field, _)| {
            let identifier = field.ident.as_ref().expect("named field");
            format_ident!("__telorgon_snapshot_{identifier}")
        })
        .collect();

    let updates = fields
        .iter()
        .zip(classified.iter())
        .zip(incoming_identifiers.iter())
        .filter_map(|((field, kind), incoming)| {
            let identifier = field.ident.as_ref().expect("named field");
            let FieldKind::Input(mode) = kind else {
                return None;
            };
            Some(match mode {
                InputMode::Compare => quote! {
                    if self.#identifier != #incoming {
                        self.#identifier = #incoming;
                        __telorgon_changed = true;
                    }
                },
                InputMode::Always => quote! {
                    self.#identifier = #incoming;
                    __telorgon_changed = true;
                },
                InputMode::CompareWith(path) => quote! {
                    if !#path(&self.#identifier, &#incoming) {
                        self.#identifier = #incoming;
                        __telorgon_changed = true;
                    }
                },
            })
        });

    let restores =
        input_fields
            .iter()
            .zip(snapshot_bindings.iter())
            .map(|((field, mode), snapshot)| {
                let identifier = field.ident.as_ref().expect("named field");
                match mode {
                    InputMode::Compare => quote! {
                        if self.#identifier != #snapshot {
                            __telorgon_mutated = true;
                        }
                        self.#identifier = #snapshot;
                    },
                    InputMode::Always => quote! {
                        self.#identifier = #snapshot;
                    },
                    InputMode::CompareWith(path) => quote! {
                        if !#path(&self.#identifier, &#snapshot) {
                            __telorgon_mutated = true;
                        }
                        self.#identifier = #snapshot;
                    },
                }
            });

    quote! {
        #item

        impl #impl_generics #fields_trait
            for #name #ty_generics #where_clause
        {
            type InputSnapshot = (#(#snapshot_types,)*);

            fn update_inputs(&mut self, incoming: Self) -> bool {
                let Self { #(#destructure,)* } = incoming;
                let mut __telorgon_changed = false;
                #(#updates)*
                __telorgon_changed
            }

            fn capture_inputs(&self) -> Self::InputSnapshot {
                (#(#snapshot_values,)*)
            }

            fn restore_inputs(&mut self, snapshot: Self::InputSnapshot) -> bool {
                let (#(#snapshot_bindings,)*) = snapshot;
                let mut __telorgon_mutated = false;
                #(#restores)*
                __telorgon_mutated
            }
        }
    }
    .into()
}

fn has_default_derive(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("derive") {
            return false;
        }
        let mut found = false;
        let _ = attribute.parse_nested_meta(|meta| {
            if meta
                .path
                .segments
                .last()
                .is_some_and(|segment| segment.ident == "Default")
            {
                found = true;
            }
            Ok(())
        });
        found
    })
}

fn parse_input_mode(attribute: &Attribute) -> syn::Result<InputMode> {
    match &attribute.meta {
        Meta::Path(_) => Ok(InputMode::Compare),
        Meta::NameValue(_) => Err(syn::Error::new_spanned(
            attribute,
            "use #[input], #[input(always)], or #[input(compare_with = path)]",
        )),
        Meta::List(list) => {
            let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
            let arguments = parser.parse2(list.tokens.clone())?;
            if arguments.len() != 1 {
                return Err(syn::Error::new_spanned(
                    list,
                    "#[input] accepts exactly one mode",
                ));
            }
            match arguments.first().expect("one argument") {
                Meta::Path(path) if path.is_ident("always") => Ok(InputMode::Always),
                Meta::NameValue(value) if value.path.is_ident("compare_with") => {
                    let syn::Expr::Path(path) = &value.value else {
                        return Err(syn::Error::new_spanned(
                            &value.value,
                            "compare_with must be a function path",
                        ));
                    };
                    Ok(InputMode::CompareWith(path.path.clone()))
                }
                other => Err(syn::Error::new_spanned(
                    other,
                    "expected always or compare_with = path",
                )),
            }
        }
    }
}

//! File parsing for the Peko checking engine.
//!
//! Every manifest style file becomes a [`Document`], a `serde_json::Value`
//! with one key path grammar over it. Build scripts become [`BuildSettings`].
//! Lockfiles become [`Dependency`] lists. The crate performs no compliance
//! analysis.

pub mod android;
pub mod bundle;
pub mod config;
pub mod deps;
pub mod error;
pub mod framework;
pub mod gradle_project;
pub mod kind;
pub mod openstep;
pub mod pbxproj;
pub mod plist_doc;
pub mod privacy_manifest;
pub mod value;

pub use android::{parse_xml, read_xml};
pub use config::{
    declares_android_application, parse_gradle, parse_pbxproj, read_config, BuildSettings,
    SettingValue,
};
pub use deps::Dependency;
pub use error::{ParseError, Result};
pub use gradle_project::{
    build_gradle_project, GradleModule, GradleProject, ModuleInput, ModuleKind, SourceSet,
};
pub use kind::{classify, FileKind, SOURCE_EXTENSIONS};
pub use pbxproj::{parse_pbxproj_project, ProductType, XcodeProject, XcodeTarget};
pub use plist_doc::{parse_plist, read_plist};
pub use value::{display_value, Document, KeyPath, Segment};

pub use self::dependency::Dependency;
pub use self::manifest::{Manifest, Target, TargetKind, Profile, LibKind, Profiles};
pub use self::manifest::{EitherManifest, VirtualManifest};
pub use self::package::{Package, PackageSet};
pub use self::package_id::PackageId;
pub use self::package_id_spec::PackageIdSpec;
pub use self::registry::Registry;
pub use self::resolver::Resolve;
pub use self::shell::{Shell, Verbosity};
pub use self::source::{Source, SourceId, SourceMap, GitReference};
pub use self::summary::Summary;
pub use self::workspace::{Workspace, WorkspaceConfig};

pub mod source;
pub mod package;
pub mod package_id;
pub mod dependency;
pub mod manifest;
pub mod resolver;
pub mod summary;
pub mod shell;
pub mod registry;
mod package_id_spec;
mod workspace;

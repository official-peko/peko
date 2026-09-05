//! Whether enough of a project was read to answer at all.
//!
//! Peko refuses to guess everywhere else. `has_64_bit_abi` returns `None`
//! rather than `false`. An undecided precondition never becomes a finding. A
//! variable the matcher cannot resolve decides nothing.
//!
//! The one place that principle was not applied is the whole project. A
//! Flutter app with an unparsed `pubspec.lock` produced a clean report built
//! on a fifth of what it links, and nothing in the report said so. This is
//! that principle at the project level.

use peko_parse::framework::Framework;

/// Why a project cannot be answered for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// The framework is one this version does not read the dependencies of.
    FrameworkNotRead(Framework),
    /// Nothing was placed, so no assumption about the layout is safe.
    Unplaceable,
    /// The framework is read, and this project still gave up almost nothing.
    NothingFound { sources: usize, dependencies: usize },
}

impl Unreadable {
    /// What to tell the person who ran it.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Unreadable::FrameworkNotRead(framework) => format!(
                "This looks like a {} project. Peko does not read the dependency graph of one yet, so a report would rest on a fraction of what the app links. Set the framework fact in .pekorc.json to override this.",
                framework.as_str()
            ),
            Unreadable::Unplaceable => "Peko could not tell what kind of project this is. It found no Xcode project, no Android manifest, and no framework manifest.".to_string(),
            Unreadable::NothingFound { sources, dependencies } => format!(
                "Peko read {sources} source files and {dependencies} dependencies here. That is too little to report on."
            ),
        }
    }
}

/// The fewest dependencies a project can declare and still be reported on.
///
/// An app with none at all is either a template or a project whose manifest
/// nobody parsed. Both deserve a refusal rather than a pass.
pub const MIN_DEPENDENCIES: usize = 1;

/// The fewest source files.
pub const MIN_SOURCES: usize = 2;

/// Decide whether a report on this project would mean anything.
///
/// # Errors
///
/// Returns the reason when it would not.
pub fn readable(
    framework: Framework,
    sources: usize,
    dependencies: usize,
) -> Result<(), Unreadable> {
    if framework == Framework::Unknown {
        return Err(Unreadable::Unplaceable);
    }
    if !framework.dependencies_are_read() {
        return Err(Unreadable::FrameworkNotRead(framework));
    }
    if sources < MIN_SOURCES && dependencies < MIN_DEPENDENCIES {
        return Err(Unreadable::NothingFound {
            sources,
            dependencies,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unity_project_is_refused_rather_than_passed() {
        // The dangerous answer is Ok. Every rule would run against a project
        // whose dependencies nobody read, and the report would say pass.
        let refusal = readable(Framework::Unity, 40, 0).expect_err("must refuse");
        assert!(refusal.message().contains("Unity") || refusal.message().contains("unity"));
    }

    #[test]
    fn a_project_nobody_could_place_is_refused() {
        assert_eq!(
            readable(Framework::Unknown, 100, 50),
            Err(Unreadable::Unplaceable)
        );
    }

    #[test]
    fn a_flutter_project_with_a_real_graph_is_read() {
        assert!(readable(Framework::Flutter, 30, 22).is_ok());
    }

    #[test]
    fn an_empty_directory_is_refused() {
        assert!(readable(Framework::Native, 0, 0).is_err());
    }

    #[test]
    fn a_native_project_with_no_dependencies_still_reads() {
        // An app that links nothing is unusual and legal. Source files are
        // the evidence that somebody wrote something.
        assert!(readable(Framework::Native, 20, 0).is_ok());
    }

    #[test]
    fn the_refusal_says_how_to_override_it() {
        let refusal = readable(Framework::Unity, 10, 0).expect_err("must refuse");
        assert!(
            refusal.message().contains(".pekorc.json"),
            "a refusal a person cannot get past is a wall"
        );
    }
}

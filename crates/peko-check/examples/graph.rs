//! Print the Gradle module graph for a project. Debug aid, not shipped.
use peko_check::{config::PekoConfig, project::Project};
use std::path::Path;

fn main() {
    for arg in std::env::args().skip(1) {
        let config = PekoConfig::new(peko_rules::Platform::Android);
        let project = Project::load(Path::new(&arg), &config).expect("load");
        let graph = &project.gradle_project;
        let apps: Vec<_> = graph
            .application_modules()
            .map(|m| m.dir.display().to_string())
            .collect();
        let libs = graph
            .modules
            .iter()
            .filter(|m| m.kind == peko_parse::ModuleKind::Library)
            .count();
        let tests = graph
            .modules
            .iter()
            .filter(|m| m.kind == peko_parse::ModuleKind::Test)
            .count();
        let other = graph
            .modules
            .iter()
            .filter(|m| m.kind == peko_parse::ModuleKind::Other)
            .count();
        let flagged = project
            .gradle_settings
            .iter()
            .filter(|c| c.is_application)
            .count();
        println!(
            "{arg}\n  modules {}  application {:?}  library {libs}  test {tests}  other {other}\n  gradle configs flagged as application: {flagged}\n  sources {}  manifests {}",
            graph.modules.len(),
            apps,
            project.sources.len(),
            project.android_manifests.len(),
        );
        for warning in project.warnings.iter().filter(|w| w.contains("skipped")) {
            println!("  {warning}");
        }
    }
}

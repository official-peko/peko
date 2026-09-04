//! Print the facts the checker reads from a project. Debug aid, not shipped.
use peko_check::{config::PekoConfig, project::Project};
use std::path::Path;

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("a project path");
    let platform = match args.next().as_deref() {
        Some("android") => peko_rules::Platform::Android,
        _ => peko_rules::Platform::Ios,
    };
    let config = PekoConfig::new(platform);
    let project = Project::load(Path::new(&path), &config).expect("load");
    println!("{path}");
    for (key, value) in &project.derived_facts {
        println!("  {key:<40} {value}");
    }
}

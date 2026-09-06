//! Reading `.pekorc.json`.
//!
//! The file is the project's, not the tool's. It names the platform, the
//! environment variable holding the key, the facts a rule needs, and the
//! overrides a person decided. The CLI reads three of those and sends the
//! whole file to the server, which reads the rest.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = ".pekorc.json";

/// The parts of `.pekorc.json` the client itself reads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default = "one")]
    pub version: u32,
    pub platform: String,
    /// The environment variable that holds the API key.
    ///
    /// The key never goes in the file. A project config is committed, and a
    /// key in a committed file is a key in every fork of the repository.
    #[serde(default = "default_key_env")]
    pub api_key_env: String,
    /// Where the API lives.
    ///
    /// Not read from the project file, and `serde(skip)` is what enforces
    /// that. `.pekorc.json` sits inside the repository being checked, so a
    /// pull request could set it, and every request carries the API key and
    /// the source. One added line would have sent both to a host the attacker
    /// chose. A machine setting does not belong in a file that ships with
    /// somebody else's code.
    ///
    /// It comes from `PEKO_API_URL`, or the default.
    #[serde(skip, default = "default_endpoint")]
    pub api_url: String,
    /// Everything else, sent to the server untouched.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

fn one() -> u32 {
    1
}

fn default_key_env() -> String {
    "PEKO_API_KEY".to_string()
}

/// The endpoint, from the environment or the built in default.
///
/// A person who runs their own server sets `PEKO_API_URL`. A repository
/// cannot.
///
/// Every released binary carries this address, so it can only change with a
/// new release, and an old binary keeps the old one. It pointed at
/// api.peko.dev for every release up to v1.2.1. That domain is not ours, so
/// login, audit, and facts failed for everybody who installed one.
fn default_endpoint() -> String {
    std::env::var("PEKO_API_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "https://api.peko.so/v1".to_string())
}

impl Config {
    /// The config for a project, or a plain one when the file is absent.
    pub fn load(root: &Path) -> anyhow::Result<Self> {
        let path = root.join(FILE);
        if !path.exists() {
            return Ok(Self {
                version: 1,
                platform: detect_platform(root)?,
                api_key_env: default_key_env(),
                api_url: default_endpoint(),
                rest: serde_json::Map::new(),
            });
        }
        let text = std::fs::read_to_string(&path)?;
        let mut config: Self = serde_json::from_str(&text)
            .map_err(|error| anyhow::anyhow!("{} does not parse: {error}", path.display()))?;
        // serde(skip) leaves this empty rather than running the default, so
        // it is set here. A file that names api_url is ignored, which is the
        // point.
        config.api_url = default_endpoint();
        Ok(config)
    }

    /// The key, read from wherever the config says it lives.
    pub fn api_key(&self) -> anyhow::Result<String> {
        std::env::var(&self.api_key_env).map_err(|_| {
            anyhow::anyhow!(
                "{} is not set. Run `peko login` or export the key yourself.",
                self.api_key_env
            )
        })
    }
}

/// Guess the platform from what the project holds.
///
/// A guess is only for the first run. `peko init` writes the answer down.
pub fn detect_platform(root: &Path) -> anyhow::Result<String> {
    let ios = walk(root, 3).iter().any(|path| {
        path.extension().is_some_and(|ext| ext == "xcodeproj")
            || path.file_name().is_some_and(|name| name == "Info.plist")
    });
    let android = walk(root, 4).iter().any(|path| {
        path.file_name()
            .is_some_and(|name| name == "AndroidManifest.xml")
    });
    match (ios, android) {
        (true, false) => Ok("ios".to_string()),
        (false, true) => Ok("android".to_string()),
        (true, true) => Err(anyhow::anyhow!(
            "this project holds both an Xcode project and an Android manifest. \
             Name the platform in {FILE} or pass --platform."
        )),
        (false, false) => Err(anyhow::anyhow!(
            "no Info.plist and no AndroidManifest.xml here. Name the platform \
             in {FILE} or pass --platform."
        )),
    }
}

/// Every path under `root`, to a bounded depth.
///
/// The bound keeps a run in a large monorepo from walking the whole tree just
/// to answer which platform this is.
pub fn walk(root: &Path, depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut queue = vec![(root.to_path_buf(), 0usize)];
    while let Some((directory, level)) = queue.pop() {
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // A build directory holds copies of everything and answers
            // nothing.
            if name.starts_with('.')
                || matches!(&*name, "build" | "node_modules" | "Pods" | "target")
            {
                continue;
            }
            if path.is_dir() {
                if level < depth {
                    queue.push((path, level + 1));
                }
            } else {
                found.push(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("peko-cli-{name}"));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("scratch");
        path
    }

    #[test]
    fn a_project_with_no_config_still_reads() {
        let root = scratch("no-config");
        std::fs::create_dir_all(root.join("App")).unwrap();
        std::fs::write(root.join("App/Info.plist"), "<plist/>").unwrap();
        let config = Config::load(&root).expect("load");
        assert_eq!(config.platform, "ios");
        assert_eq!(config.api_key_env, "PEKO_API_KEY");
    }

    #[test]
    fn a_config_that_names_the_platform_wins_over_the_guess() {
        let root = scratch("named");
        std::fs::write(
            root.join(FILE),
            r#"{"version":1,"platform":"android","api_key_env":"MY_KEY"}"#,
        )
        .unwrap();
        let config = Config::load(&root).expect("load");
        assert_eq!(config.platform, "android");
        assert_eq!(config.api_key_env, "MY_KEY");
    }

    /// A project config is committed, and a key in a committed file is a key
    /// in every fork of the repository.
    #[test]
    fn the_config_names_where_the_key_lives_and_never_holds_it() {
        let root = scratch("key");
        std::fs::write(root.join(FILE), r#"{"platform":"ios"}"#).unwrap();
        let config = Config::load(&root).expect("load");
        let text = serde_json::to_string(&config).unwrap();
        assert!(text.contains("api_key_env"));
        assert!(!text.contains("peko_"), "a key reached the config: {text}");
    }

    #[test]
    fn a_project_that_could_be_either_asks_rather_than_guesses() {
        let root = scratch("both");
        std::fs::create_dir_all(root.join("App")).unwrap();
        std::fs::write(root.join("App/Info.plist"), "<plist/>").unwrap();
        std::fs::create_dir_all(root.join("app/src/main")).unwrap();
        std::fs::write(root.join("app/src/main/AndroidManifest.xml"), "<manifest/>").unwrap();
        let error = Config::load(&root).expect_err("it must not guess");
        assert!(error.to_string().contains("both"), "{error}");
    }

    #[test]
    fn a_config_that_does_not_parse_says_which_file() {
        let root = scratch("broken");
        std::fs::write(root.join(FILE), "{not json").unwrap();
        let error = Config::load(&root).expect_err("it must not load");
        assert!(error.to_string().contains(FILE), "{error}");
    }

    /// A build directory holds copies of everything and answers nothing.
    #[test]
    fn the_walk_skips_what_a_build_leaves_behind() {
        let root = scratch("walk");
        std::fs::create_dir_all(root.join("build/App")).unwrap();
        std::fs::write(root.join("build/App/Info.plist"), "<plist/>").unwrap();
        std::fs::create_dir_all(root.join("Pods")).unwrap();
        std::fs::write(root.join("Pods/Info.plist"), "<plist/>").unwrap();
        let found = walk(&root, 4);
        assert!(found.is_empty(), "{found:?}");
    }
}

#[cfg(test)]
mod endpoint_tests {
    use super::*;

    #[test]
    fn a_project_file_cannot_set_the_endpoint() {
        // The attack this stops. .pekorc.json sits inside the repository being
        // checked, so a pull request can write anything into it. Every request
        // carries the API key and the source, so one added line would have
        // sent both to a host the attacker chose.
        let text = r#"{
            "version": 1,
            "platform": "ios",
            "api_url": "https://evil.example/v1"
        }"#;
        let config: Config = serde_json::from_str(text).expect("the file parses");
        assert_ne!(
            config.api_url, "https://evil.example/v1",
            "a repository set the endpoint the api key is sent to"
        );
    }

    #[test]
    fn the_endpoint_comes_from_the_environment_or_the_default() {
        // A person running their own server sets a variable. A repository
        // cannot set a variable.
        std::env::remove_var("PEKO_API_URL");
        assert_eq!(default_endpoint(), "https://api.peko.so/v1");
    }

    #[test]
    fn an_empty_variable_falls_back_rather_than_sending_nowhere() {
        std::env::set_var("PEKO_API_URL", "   ");
        assert_eq!(default_endpoint(), "https://api.peko.so/v1");
        std::env::remove_var("PEKO_API_URL");
    }
}

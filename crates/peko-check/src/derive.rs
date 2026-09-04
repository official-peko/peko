//! Facts the checker reads from the project.
//!
//! A precondition can turn on something no file states, and those come from
//! `.pekorc.json`. Many of them are not that kind of thing at all. Whether the
//! code calls `MusicKit`, which Android API level the build targets, whether
//! the project builds a Safari extension: every one of those is in the files
//! already. Asking a developer for them is asking them to do the checker's
//! job, and their answer is worth less than the file.
//!
//! So the checker reads them. A fact a person declares still wins, because a
//! person sometimes knows the code is dead and the checker never does.

use crate::project::Project;
use peko_parse::ProductType;
use peko_rules::facts::{self, Kind};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// A symbol that, when the code holds it, makes a fact true.
struct Signal {
    fact: &'static str,
    /// Any one of these in a source file settles it.
    symbols: &'static [&'static str],
}

/// The symbol scans. Each one reads as a question about the code.
const SIGNALS: &[Signal] = &[
    Signal {
        fact: "uses_apple_music",
        symbols: &[
            "MusicKit",
            "MPMediaLibrary",
            "SKCloudServiceController",
            "MPMusicPlayer",
        ],
    },
    Signal {
        fact: "uses_apple_pay",
        symbols: &["PKPaymentRequest", "PKPaymentAuthorization", "PassKit"],
    },
    Signal {
        fact: "uses_game_center",
        symbols: &["GKLocalPlayer", "GKGameCenter", "import GameKit"],
    },
    Signal {
        fact: "uses_weatherkit",
        symbols: &["WeatherKit", "WeatherService"],
    },
    Signal {
        fact: "uses_health_connect",
        symbols: &["HealthConnectClient", "androidx.health.connect"],
    },
    Signal {
        fact: "uses_social_login",
        symbols: &[
            "GIDSignIn",
            "LoginManager",
            "FBSDKLoginKit",
            "ASAuthorizationAppleIDProvider",
            "com.facebook.login",
            "GoogleSignIn",
        ],
    },
    Signal {
        fact: "supports_matter",
        symbols: &[
            "MatterSupport",
            "MTRDevice",
            "com.google.android.gms.home.matter",
        ],
    },
    Signal {
        fact: "uses_user_initiated_data_transfer_jobs",
        symbols: &["setUserInitiated", "JobInfo.Builder", "UserInitiatedJob"],
    },
    // The facts below were questions on the developer's form until the shape
    // of that form was measured. 57 questions, and 35 of them switched on one
    // rule apiece. Every one the project can answer is one the form does not
    // have to ask.
    Signal {
        fact: "ships_vpn",
        symbols: &[
            "NEVPNManager",
            "NEPacketTunnelProvider",
            "NETunnelProviderManager",
            "android.net.VpnService",
            "VpnService.Builder",
        ],
    },
    Signal {
        fact: "has_in_app_purchases",
        symbols: &[
            "SKPaymentQueue",
            "StoreKit.Product",
            "Product.products",
            "com.android.billingclient",
            "BillingClient",
        ],
    },
    Signal {
        fact: "ships_mdm",
        symbols: &[
            "DeviceActivity",
            "ManagedSettings",
            "DevicePolicyManager",
            "DeviceAdminReceiver",
        ],
    },
    Signal {
        fact: "alternative_marketplace_app",
        symbols: &[
            "PackageInstaller.Session",
            "packageInstaller.createSession",
            "AppStoreConnect",
            "MarketplaceKit",
        ],
    },
    Signal {
        fact: "is_remote_desktop_client",
        symbols: &[
            "RFBClient",
            "VncClient",
            "RdpClient",
            "FreeRDP",
            "libvncclient",
        ],
    },
];

/// A fact that a flagged dependency answers.
struct DependencySignal {
    fact: &'static str,
    flag: peko_rules::DependencyFlagType,
}

/// The knowledge base already says which packages advertise and which track.
/// Asking a developer whether the app shows ads, when the lockfile names an ad
/// SDK, is asking a question the project answered first.
const DEPENDENCY_SIGNALS: &[DependencySignal] = &[DependencySignal {
    fact: "shows_ads",
    flag: peko_rules::DependencyFlagType::Tracking,
}];

/// Read every fact the project can answer for itself.
///
/// A fact stays out of the map when the project gives no answer, so it reports
/// undecided rather than a guess. `false` is an answer, and a wrong `false`
/// silences a rule that should have run.
pub fn derive(
    project: &Project,
    config: Option<&crate::config::PekoConfig>,
    knowledge: Option<&crate::knowledge::KnowledgeBase>,
) -> (BTreeMap<String, Value>, Vec<String>, Vec<String>) {
    let mut found: BTreeMap<String, Value> = BTreeMap::new();

    found.insert("platform".to_string(), json!(project.platform.to_string()));

    if let Some(level) = android_target_sdk(project) {
        found.insert("target_sdk_version".to_string(), json!(level));
    }

    // A target graph settles these, and nothing else does. Without a parsed
    // project file the checker cannot say the target is absent, only that it
    // did not see one, so the fact stays out.
    if !project.xcode_projects.is_empty() {
        found.insert(
            "has_safari_extension".to_string(),
            json!(has_target_named(project, "safari")),
        );
        found.insert(
            "has_app_clip".to_string(),
            json!(has_target_named(project, "clip")),
        );
    }

    // A dependency the knowledge base flags answers a question the form used
    // to ask. Without the knowledge base the fact stays unknown rather than
    // false, because a project with no lockfile read is not a project with no
    // advertising.
    if let Some(base) = knowledge {
        for signal in DEPENDENCY_SIGNALS {
            let flagged = project.dependencies.iter().any(|dependency| {
                base.get(&dependency.package_id).is_some_and(|entry| {
                    entry
                        .compliance_flags
                        .iter()
                        .any(|flag| flag.flag_type == signal.flag)
                })
            });
            if flagged {
                found.insert(signal.fact.to_string(), json!(true));
            }
        }
    }

    if !project.sources.is_empty() {
        for signal in SIGNALS {
            let present = project.sources.iter().any(|file| {
                signal
                    .symbols
                    .iter()
                    .any(|symbol| file.text().contains(symbol))
            });
            found.insert(signal.fact.to_string(), json!(present));
        }
    }

    // What the code says, before what the vocabulary assumes. An inference is
    // a guess from evidence, so it joins the assumption list and the report
    // prints it. A reader who disagrees answers the fact and the answer wins.
    let mut inferred = infer_from_code(project, knowledge, &mut found);
    inferred.extend(infer_from_facts(config, &mut found));
    let mut assumed = inferred.clone();

    // A fact with a registered default is answered, not asked. The default is
    // never silent: `assumed_facts` names every one, and the report prints
    // them, so an answer that rested on a guess says so.
    for fact in facts::FACTS {
        if fact.kind != Kind::Declared {
            continue;
        }
        let Some(text) = fact.default else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        if !found.contains_key(fact.name) {
            found.insert(fact.name.to_string(), value);
            assumed.push(fact.name.to_string());
        }
    }

    debug_assert!(
        found.keys().all(|key| facts::lookup(key).is_some()),
        "every fact the checker fills in must be in the vocabulary"
    );
    (found, assumed, inferred)
}

/// A fact the code answers only when the answer is yes.
///
/// The direction matters more than the signals. A wrong `true` makes a rule
/// run that did not need to, and the corpus catches that within a day. A wrong
/// `false` makes a rule stay silent, and nothing catches it at all: the report
/// looks clean and the finding never appears.
///
/// So every signal here reads one way. Seeing a sign up screen says the app
/// has accounts. Seeing none says nothing, because the screen may be written
/// in a file this scan did not read, or drawn from a server. When the evidence
/// is absent the fact stays unanswered and a person is asked.
struct Inference {
    fact: &'static str,
    /// Any one of these in the source settles it as true.
    symbols: &'static [&'static str],
}

const INFERENCES: &[Inference] = &[
    Inference {
        fact: "has_user_accounts",
        symbols: &[
            "signUp",
            "signup",
            "createAccount",
            "registerUser",
            "SignUpView",
            "RegistrationActivity",
            "createUserWithEmail",
        ],
    },
    Inference {
        fact: "shows_consent_ui",
        symbols: &[
            "ConsentForm",
            "UserMessagingPlatform",
            "ConsentInformation",
            "CMPConsent",
            "requestTrackingAuthorization",
            "ATTrackingManager.requestTrackingAuthorization",
        ],
    },
    Inference {
        fact: "collects_birthdate",
        symbols: &[
            "dateOfBirth",
            "date_of_birth",
            "birthDate",
            "birthday",
            "DatePicker(\"Date of birth\"",
        ],
    },
    Inference {
        fact: "has_user_generated_content",
        symbols: &[
            "func post(",
            "createPost",
            "submitComment",
            "uploadMedia",
            "PostComposer",
            "CommentView",
            "sendMessage",
        ],
    },
];

/// Read the facts the project answers for itself, and say which are guesses.
///
/// A fact here is inferred rather than measured, so the report names it the
/// same way it names a default. A reader who disagrees answers it in
/// `.pekorc.json` and the answer wins.
fn infer_from_code(
    project: &Project,
    knowledge: Option<&crate::knowledge::KnowledgeBase>,
    found: &mut BTreeMap<String, Value>,
) -> Vec<String> {
    let mut guessed = Vec::new();
    let set = |found: &mut BTreeMap<String, Value>,
               guessed: &mut Vec<String>,
               name: &str,
               value: Value| {
        if found.insert(name.to_string(), value).is_none() {
            guessed.push(name.to_string());
        }
    };

    for inference in INFERENCES {
        let present = project.sources.iter().any(|file| {
            inference
                .symbols
                .iter()
                .any(|symbol| file.text().contains(symbol))
        });
        if present {
            set(found, &mut guessed, inference.fact, json!(true));
        }
    }

    // A dependency that collects data is a third party that processes it, and
    // the knowledge base already says which packages do. An advertising SDK
    // shares what it collects, which is what the CCPA calls sharing.
    if let Some(base) = knowledge {
        let mut processes = false;
        let mut shares = false;
        for dependency in &project.dependencies {
            let Some(entry) = base.get(&dependency.package_id) else {
                continue;
            };
            for flag in &entry.compliance_flags {
                match flag.flag_type {
                    peko_rules::DependencyFlagType::DataCollection => processes = true,
                    peko_rules::DependencyFlagType::Tracking => {
                        processes = true;
                        shares = true;
                    }
                    _ => {}
                }
            }
        }
        if processes {
            set(
                found,
                &mut guessed,
                "uses_third_party_processor",
                json!(true),
            );
            set(found, &mut guessed, "collects_personal_data", json!(true));
        }
        if shares {
            set(
                found,
                &mut guessed,
                "sells_or_shares_personal_information",
                json!(true),
            );
        }
    }

    // An app that signs people in holds personal data about them.
    if found.get("has_user_accounts") == Some(&json!(true)) {
        set(found, &mut guessed, "collects_personal_data", json!(true));
    }

    // A privacy policy in the repository answers where it is.
    if let Some(path) = project.policy_documents.first() {
        set(
            found,
            &mut guessed,
            "privacy_policy_document_path",
            json!(path.to_string_lossy()),
        );
    }

    guessed
}

/// Facts that follow from other facts.
///
/// One answer settles several. Somebody shipping to the European Union and
/// holding personal data needs consent, and asking them a second question to
/// learn that wastes their time.
fn infer_from_facts(
    config: Option<&crate::config::PekoConfig>,
    found: &mut BTreeMap<String, Value>,
) -> Vec<String> {
    let read = |key: &str| -> Option<Value> {
        config
            .and_then(|config| config.fact(key).cloned())
            .or_else(|| found.get(key).cloned())
    };

    let in_eu = match read("distributes_in") {
        Some(Value::Array(places)) => places
            .iter()
            .filter_map(Value::as_str)
            .any(|place| place.eq_ignore_ascii_case("eu")),
        Some(Value::String(place)) => place.eq_ignore_ascii_case("eu"),
        _ => false,
    };
    let collects = read("collects_personal_data") == Some(json!(true));

    if in_eu && collects && !found.contains_key("requires_consent") {
        found.insert("requires_consent".to_string(), json!(true));
        return vec!["requires_consent".to_string()];
    }
    Vec::new()
}

/// The Android API level the build targets.
fn android_target_sdk(project: &Project) -> Option<i64> {
    project
        .gradle_settings
        .iter()
        .filter(|entry| entry.is_application)
        .find_map(|entry| {
            entry
                .settings
                .first("targetSdkVersion")
                .or_else(|| entry.settings.first("targetSdk"))
                .and_then(|value| value.value.parse::<i64>().ok())
        })
}

/// True when the project builds a target whose name holds `needle`.
fn has_target_named(project: &Project, needle: &str) -> bool {
    project.xcode_projects.iter().any(|parsed| {
        parsed.targets.iter().any(|target| {
            matches!(
                target.product_type,
                ProductType::AppExtension | ProductType::Other(_)
            ) && target.name.to_ascii_lowercase().contains(needle)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use peko_rules::Platform;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn the_android_api_level_comes_from_the_build_file() {
        let config = crate::config::PekoConfig::new(Platform::Android);
        let project =
            Project::load(&root().join("fixtures/android-multi-module"), &config).unwrap();
        assert_eq!(
            project.derived_facts.get("target_sdk_version"),
            Some(&json!(34))
        );
    }

    #[test]
    fn a_declared_answer_beats_the_derived_one() {
        let mut config = crate::config::PekoConfig::new(Platform::Android);
        config
            .facts
            .insert("target_sdk_version".to_string(), json!(21));
        let project =
            Project::load(&root().join("fixtures/android-multi-module"), &config).unwrap();
        // The project still reads 34 from the build file. Precedence lives in
        // the lookup, so the declared 21 is what a precondition sees.
        assert_eq!(
            project.derived_facts.get("target_sdk_version"),
            Some(&json!(34))
        );
        assert_eq!(
            project.fact("target_sdk_version", &config),
            Some(&json!(21))
        );
    }

    #[test]
    fn every_derived_name_is_registered_as_derived() {
        for signal in SIGNALS {
            let fact = facts::lookup(signal.fact)
                .unwrap_or_else(|| panic!("{} is not in the vocabulary", signal.fact));
            assert_eq!(fact.kind, Kind::Derived, "{}", signal.fact);
        }
    }
}

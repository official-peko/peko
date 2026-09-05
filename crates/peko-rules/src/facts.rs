//! The vocabulary of facts a rule may name.
//!
//! A precondition can turn on a fact that no file states: whether the app sits
//! in the Kids Category, where it ships, whether the developer is an approved
//! nonprofit. Those come from `.pekorc.json`.
//!
//! Nothing used to check a fact name. The compiler wrote a new one for almost
//! every rule it produced, and the result was 75 names for maybe 50 ideas:
//! five different spellings of "this app has user accounts", two of "this app
//! is a remote desktop client", two of "this app uses sensitive personal
//! information beyond the permitted purposes". A rule naming a fact that no
//! developer will ever declare is a rule that never fires. Around 40 percent
//! of the interpretive set sat dormant that way.
//!
//! So the vocabulary is closed. A rule that names a fact outside this registry
//! fails validation, and the compiler is given the list.
//!
//! A fact is one of two kinds, and the split matters more than the names. A
//! [`Kind::Derived`] fact is one the checker reads from the project itself, so
//! nobody is asked for it. A [`Kind::Declared`] fact is one only a person
//! knows. Every fact that the project can answer belongs in the first group:
//! asking a developer whether their own code calls `MusicKit` is asking them
//! to do the checker's job, and their answer is worth less than the file.

use std::collections::BTreeMap;
use std::sync::OnceLock;

/// Who answers a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// A person answers it in `.pekorc.json`.
    Declared,
    /// The checker reads it from the project.
    Derived,
}

/// What shape a fact's value takes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Bool,
    Text,
    Integer,
    /// A list of short codes, for example the places an app ships.
    TextList,
}

/// One fact a rule may name.
#[derive(Debug, Clone, Copy)]
pub struct Fact {
    pub name: &'static str,
    pub kind: Kind,
    pub shape: Shape,
    /// What the fact means, put as the question a developer answers.
    pub question: &'static str,
    /// Names that earlier compiles produced for the same idea.
    pub aliases: &'static [&'static str],
    /// What to assume when nobody answers, written as JSON.
    ///
    /// A default is only right where one answer is overwhelmingly the common
    /// one and a wrong guess costs little. It is never silent: the report
    /// names every fact it assumed, so a developer can see what the answer
    /// rested on and correct it.
    ///
    /// Two facts have no default on purpose. Where an app ships and whether it
    /// is for children each decide dozens of rules, and guessing either one
    /// would decide them wrongly at scale.
    pub default: Option<&'static str>,
}

/// The whole vocabulary.
pub const FACTS: &[Fact] = &[
    // --- Where the app ships, and what it is ---------------------------
    Fact {
        name: "distributes_in",
        kind: Kind::Declared,
        shape: Shape::TextList,
        question: "Which places does the app ship in? Use `eu`, `US`, `US-CA`.",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "platform",
        kind: Kind::Derived,
        shape: Shape::Text,
        question: "Which platform does this target build for?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "framework",
        kind: Kind::Derived,
        shape: Shape::Text,
        question: "Which framework built this app?",
        aliases: &["cross_platform_framework"],
        // No default. A guess of "native" would run every native rule against
        // a project nobody read, and report a pass. Undecided is the honest
        // answer and it keeps a gated rule silent.
        default: None,
    },
    Fact {
        name: "mac_app_store",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app ship on the Mac App Store?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        // GPLAY-API-001 fired on 41 of 346 published apps, and every one of
        // them is listed on Google Play today. The floor blocks a submission
        // and an update. It does not remove an app already listed, so an app
        // sits at an old target level indefinitely and breaks nothing.
        //
        // The fact has no default on purpose. Undeclared reports undecided,
        // which is silence rather than a pass, and a developer who is about to
        // ship answers it once.
        name: "submitting_to_store",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Are you preparing a build to upload to the store? \
                   Some rules apply to a submission and not to an app that \
                   already ships.",
        aliases: &["submitting", "preparing_submission", "shipping_an_update"],
        default: None,
    },
    Fact {
        name: "kids_category",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the app in the Kids Category, or directed at children?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "approved_nonprofit",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the developer an approved nonprofit?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "permanently_private_app",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the app permanently private, so it never reaches public listing?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "medical_app_category",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app make a medical claim, or act as a medical device?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "medical_app_has_regulatory_clearance",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the medical function hold regulatory clearance?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "offers_real_money_gaming",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app offer real money gaming, gambling, or lotteries?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "ships_crypto_exchange",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app run a cryptocurrency exchange?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "ships_vpn",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app provide a VPN service?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "ships_mdm",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app provide mobile device management?",
        aliases: &["offers_mdm_services"],
        default: None,
    },
    Fact {
        name: "is_remote_desktop_client",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Is the app a remote desktop or remote control client?",
        aliases: &["remote_desktop_app"],
        default: None,
    },
    Fact {
        name: "alternative_marketplace_app",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Is the app an alternative app marketplace?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "education_or_enterprise_app",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app ship only to a school or a company?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_government_id_login",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does sign in use a government issued identity?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "is_third_party_service_client",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the app a client for a service that someone else runs?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "tv_or_xr_app",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app ship on Android TV or an XR headset?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "is_android_container_app",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the app a container that loads another app or a website?",
        aliases: &[],
        default: Some("false"),
    },
    // --- What the app hosts --------------------------------------------
    Fact {
        name: "has_user_generated_content",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Can one user see content that another user made?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "has_web_sourced_ugc",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app show user content pulled from the open web?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "hosts_mini_apps",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app host mini apps, mini games, or plug-ins?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "hosts_creator_content",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app host content that creators publish and sell?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "shows_ads",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app show advertising?",
        aliases: &[],
        default: None,
    },
    // --- Accounts -------------------------------------------------------
    Fact {
        name: "has_user_accounts",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Can a user create an account in the app?",
        aliases: &[
            "allows_account_creation",
            "allows_in_app_account_creation",
            "has_account_creation",
            "offers_account_creation",
        ],
        default: None,
    },
    Fact {
        name: "uses_own_account_system_only",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app use only its own sign in, with no third party option?",
        aliases: &[],
        default: Some("false"),
    },
    // --- Money ----------------------------------------------------------
    Fact {
        name: "has_in_app_purchases",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app sell anything through in-app purchase?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "promotes_in_app_purchases",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app promote in-app purchases on the store page?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "promotes_in_app_events",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app publish in-app events on the store page?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "offers_free_trial",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app offer a free trial?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_preorder",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app ship as a pre-order?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_external_purchase_link",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app link out to a purchase page under an entitlement?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "qualifies_3_1_3_other_purchase_methods",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app qualify under guideline 3.1.3 for other purchase methods?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "offers_financial_incentive",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app offer a price difference in return for personal data?",
        aliases: &[],
        default: Some("false"),
    },
    // --- Data handling --------------------------------------------------
    Fact {
        name: "collects_personal_data",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app collect personal data?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "collects_birthdate",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app collect a birthdate or an age?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "collects_data_not_from_user",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app obtain personal data from anywhere but the user?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "sells_or_shares_personal_information",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the business sell or share personal information?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_sensitive_personal_information",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app use sensitive personal information?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_spi_beyond_permitted_purposes",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does that use go beyond the purposes the CCPA permits?",
        aliases: &["uses_sensitive_personal_information_beyond_permitted_purposes"],
        default: Some("false"),
    },
    Fact {
        name: "reuses_data_for_new_purpose",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app reuse collected data for a purpose it did not disclose?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_third_party_processor",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does another company process personal data for you?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "transfers_data_outside_eu",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does personal data leave the European Economic Area?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "data_retention_days",
        kind: Kind::Declared,
        shape: Shape::Integer,
        question: "How many days do you keep personal data?",
        aliases: &[],
        default: None,
    },
    // --- Consent and legal basis -----------------------------------------
    Fact {
        name: "requires_consent",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app need consent before it processes personal data?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "shows_consent_ui",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app ask for that consent in the interface?",
        aliases: &["obtains_consent_for_data_processing"],
        default: None,
    },
    Fact {
        name: "processing_basis_consent_or_contract",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the legal basis for processing consent or a contract?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "processes_special_category_data",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app process special category or criminal offence data?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "processing_is_occasional",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Is the processing occasional rather than routine?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "processing_risks_rights",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the processing risk the rights and freedoms of a person?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_admt",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the app make a significant decision about a person automatically?",
        aliases: &["uses_automated_decision_making"],
        default: Some("false"),
    },
    Fact {
        name: "employee_count_over_250",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Does the organisation employ more than 250 people?",
        aliases: &[],
        default: Some("false"),
    },
    Fact {
        name: "uses_alternative_optout_link",
        kind: Kind::Declared,
        shape: Shape::Bool,
        question: "Do you use the single combined opt-out link instead of two links?",
        aliases: &[],
        default: Some("false"),
    },
    // --- Documents you point at ------------------------------------------
    Fact {
        name: "privacy_policy_url",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "Where is the privacy policy published?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "privacy_policy_document_path",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "Where in this repository does the privacy policy live?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "parental_consent_method",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "How do you obtain verifiable parental consent?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "data_breach_notification_process",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "Where is your data breach notification process written down?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "gdpr_certification_mechanism",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "Which GDPR certification or code of conduct do you hold?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "records_of_processing_document_url",
        kind: Kind::Declared,
        shape: Shape::Text,
        question: "Where is your record of processing activities published?",
        aliases: &[],
        default: None,
    },
    // --- Facts the checker reads for itself -------------------------------
    Fact {
        name: "target_sdk_version",
        kind: Kind::Derived,
        shape: Shape::Integer,
        question: "Which Android API level does the app target?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "has_safari_extension",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the project build a Safari extension target?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "has_app_clip",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the project build an App Clip target?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_apple_music",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code call MusicKit or the media library?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_apple_pay",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code call Apple Pay?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_game_center",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code call Game Center?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_weatherkit",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code call WeatherKit?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_health_connect",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code call Health Connect?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_social_login",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the code sign a user in through a social network?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "supports_matter",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app support the Matter smart home standard?",
        aliases: &[],
        default: None,
    },
    Fact {
        name: "uses_user_initiated_data_transfer_jobs",
        kind: Kind::Derived,
        shape: Shape::Bool,
        question: "Does the app run user initiated data transfer jobs?",
        aliases: &[],
        default: None,
    },
];

fn index() -> &'static BTreeMap<&'static str, &'static Fact> {
    static INDEX: OnceLock<BTreeMap<&'static str, &'static Fact>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut map = BTreeMap::new();
        for fact in FACTS {
            map.insert(fact.name, fact);
            for alias in fact.aliases {
                map.insert(*alias, fact);
            }
        }
        map
    })
}

/// The fact a name refers to, following an alias to its canonical entry.
pub fn lookup(name: &str) -> Option<&'static Fact> {
    index().get(name).copied()
}

/// The canonical name for a fact, or `None` when nothing knows the name.
pub fn canonical(name: &str) -> Option<&'static str> {
    lookup(name).map(|fact| fact.name)
}

/// True when the name is an alias rather than the canonical name.
pub fn is_alias(name: &str) -> bool {
    canonical(name).is_some_and(|found| found != name)
}

/// Every fact a person has to answer.
pub fn declared() -> impl Iterator<Item = &'static Fact> {
    FACTS.iter().filter(|fact| fact.kind == Kind::Declared)
}

/// Every fact the checker reads from the project.
pub fn derived() -> impl Iterator<Item = &'static Fact> {
    FACTS.iter().filter(|fact| fact.kind == Kind::Derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_name_and_alias_is_unique() {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        for fact in FACTS {
            assert!(seen.insert(fact.name), "{} appears twice", fact.name);
            for alias in fact.aliases {
                assert!(seen.insert(alias), "{alias} appears twice");
            }
        }
    }

    #[test]
    fn an_alias_resolves_to_its_canonical_name() {
        assert_eq!(
            canonical("allows_account_creation"),
            Some("has_user_accounts")
        );
        assert_eq!(canonical("offers_mdm_services"), Some("ships_mdm"));
        assert_eq!(
            canonical("remote_desktop_app"),
            Some("is_remote_desktop_client")
        );
        assert!(is_alias("has_account_creation"));
        assert!(!is_alias("has_user_accounts"));
    }

    #[test]
    fn a_name_nobody_registered_is_unknown() {
        assert_eq!(canonical("invented_by_a_compiler"), None);
    }

    #[test]
    fn a_fact_the_project_answers_is_never_asked_of_a_person() {
        // Asking a developer whether their own code calls MusicKit is asking
        // them to do the checker's job, and their answer is worth less than
        // the file.
        for name in [
            "target_sdk_version",
            "uses_apple_music",
            "has_safari_extension",
        ] {
            assert_eq!(lookup(name).map(|f| f.kind), Some(Kind::Derived), "{name}");
        }
    }

    #[test]
    fn every_fact_carries_a_question() {
        for fact in FACTS {
            assert!(!fact.question.is_empty(), "{} has no question", fact.name);
            // A question may be followed by a sentence of examples, so the
            // mark does not have to be the last character.
            assert!(
                fact.question.contains('?'),
                "{} is not put as a question",
                fact.name
            );
        }
    }
}

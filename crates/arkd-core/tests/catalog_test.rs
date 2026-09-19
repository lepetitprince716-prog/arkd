#![cfg(feature = "fake")]

use std::collections::BTreeSet;

use arkd_core::catalog;
use arkd_core::error::Error;

const EXPECTED: &[&str] = &[
    "StartUp",
    "CloseDown",
    "Fight",
    "Recruit",
    "Infrast",
    "Mall",
    "Award",
    "SwitchTheme",
    "Roguelike",
    "Copilot",
    "SSSCopilot",
    "ParadoxCopilot",
    "Depot",
    "OperBox",
    "Reclamation",
    "Custom",
    "SingleStep",
    "VideoRecognition",
];

#[test]
fn catalog_covers_the_protocol_task_types() {
    let names: BTreeSet<&str> = catalog::names().collect();
    let expected: BTreeSet<&str> = EXPECTED.iter().copied().collect();
    assert_eq!(names, expected);
}

#[test]
fn every_example_validates_against_its_own_schema() {
    for name in catalog::names() {
        let spec = catalog::get(name).unwrap();
        let (canonical, params) = catalog::validate(name, spec.example.clone()).unwrap();
        assert_eq!(canonical, name);
        assert_eq!(params, spec.example);
    }
}

#[test]
fn every_schema_is_well_formed() {
    for name in catalog::names() {
        let schema = &catalog::get(name).unwrap().schema;
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"].get("enable").is_some());
        assert_eq!(schema["additionalProperties"], false);
        let props: BTreeSet<&str> = schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        for req in schema["required"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            assert!(props.contains(req.as_str().unwrap()));
        }
    }
}

#[test]
fn task_type_lookup_is_case_insensitive() {
    assert_eq!(catalog::resolve("fight").unwrap(), "Fight");
    assert_eq!(catalog::resolve("  ROGUELIKE ").unwrap(), "Roguelike");
}

#[test]
fn unknown_task_type_suggests_a_near_match() {
    let err = catalog::resolve("fite").unwrap_err();
    assert!(matches!(err, Error::UnknownTaskType(_)));
    assert!(err.to_string().contains("Fight"), "{err}");
}

#[test]
fn unknown_field_is_rejected_with_a_suggestion() {
    let err = catalog::validate("Fight", serde_json::json!({"stagee": "1-7"})).unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("'stagee' is not a parameter of Fight"),
        "{msg}"
    );
    assert!(msg.contains("Did you mean 'stage'"), "{msg}");
}

#[test]
fn missing_required_field_is_reported() {
    let err = catalog::validate("StartUp", serde_json::json!({})).unwrap_err();
    assert!(
        err.to_string()
            .contains("'client_type' is required for StartUp"),
        "{err}"
    );
}

#[test]
fn wrong_type_is_reported() {
    let err = catalog::validate("Fight", serde_json::json!({"times": "five"})).unwrap_err();
    assert!(
        err.to_string().contains("'times' must be a integer"),
        "{err}"
    );
}

#[test]
fn booleans_are_not_accepted_as_numbers() {
    assert!(catalog::validate("Fight", serde_json::json!({"medicine": true})).is_err());
}

#[test]
fn enum_violation_names_the_valid_values() {
    let err =
        catalog::validate("StartUp", serde_json::json!({"client_type": "Global"})).unwrap_err();
    assert!(err.to_string().contains("must be one of"), "{err}");
}

#[test]
fn out_of_range_number_is_reported() {
    let err = catalog::validate(
        "Infrast",
        serde_json::json!({"facility": ["Mfg"], "threshold": 1.5}),
    )
    .unwrap_err();
    assert!(
        err.to_string().contains("'threshold' must be <= 1"),
        "{err}"
    );
}

#[test]
fn array_item_types_are_checked() {
    let err = catalog::validate(
        "Infrast",
        serde_json::json!({"facility": ["Mfg", "Kitchen"]}),
    )
    .unwrap_err();
    assert!(err.to_string().contains("facility[1]"), "{err}");
}

#[test]
fn all_problems_are_reported_at_once() {
    let err = catalog::validate(
        "Fight",
        serde_json::json!({"times": "5", "medicine": -1, "nope": 1}),
    )
    .unwrap_err();
    assert!(err.to_string().starts_with("3 problem(s)"), "{err}");
}

#[test]
fn non_object_params_are_rejected() {
    assert!(catalog::validate("Fight", serde_json::json!(["1-7"])).is_err());
}

#[test]
fn catalog_summary_lists_every_task() {
    let summary = catalog::list();
    assert_eq!(summary.len(), EXPECTED.len());
    let startup = summary.iter().find(|e| e.task_type == "StartUp").unwrap();
    assert_eq!(startup.required, "client_type");
    let depot = summary.iter().find(|e| e.task_type == "Depot").unwrap();
    assert_eq!(depot.required, "none");
}

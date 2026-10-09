//! Scoped models: `scopedModels` storage, the migration from `enabledModels`
//! and `modelCategories`, project replace, and alias validation (scoped-models
//! design, decisions 4, 15, 18, 19, 20).

use std::path::{Path, PathBuf};

use hoocode_code_settings::{
    Error, FileSettingsStorage, ModelCategoryName, ScopedModel, SettingsManager, SettingsScope,
};
use serde_json::{json, Value};

struct Dirs {
    _tmp: tempfile::TempDir,
    agent: PathBuf,
    project: PathBuf,
}

impl Dirs {
    fn new() -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let project = tmp.path().join("project");
        std::fs::create_dir_all(&agent).unwrap();
        std::fs::create_dir_all(project.join(".hoocode")).unwrap();
        Self {
            _tmp: tmp,
            agent,
            project,
        }
    }

    fn global_path(&self) -> PathBuf {
        FileSettingsStorage::new(&self.project, &self.agent)
            .path(SettingsScope::Global)
            .to_path_buf()
    }

    fn project_path(&self) -> PathBuf {
        FileSettingsStorage::new(&self.project, &self.agent)
            .path(SettingsScope::Project)
            .to_path_buf()
    }

    fn write_global(&self, value: Value) {
        write_json(&self.global_path(), &value);
    }

    /// The global file, or `{}` when nothing was ever written.
    fn read_global(&self) -> Value {
        match std::fs::read_to_string(self.global_path()) {
            Ok(text) => serde_json::from_str(&text).unwrap(),
            Err(_) => json!({}),
        }
    }

    fn manager(&self) -> SettingsManager {
        SettingsManager::create(&self.project, &self.agent)
    }
}

fn write_json(path: &Path, value: &Value) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_string_pretty(value).unwrap()).unwrap();
}

fn entry(model: &str) -> ScopedModel {
    ScopedModel {
        model: model.into(),
        effort: None,
        category: None,
        alias: None,
    }
}

#[test]
fn category_names_parse_and_order_cheap_to_capable() {
    use ModelCategoryName::*;
    assert!(Cheap < Fast && Fast < Standard && Standard < Capable);
    assert_eq!(ModelCategoryName::ALL, &[Cheap, Fast, Standard, Capable]);
    for name in ModelCategoryName::ALL {
        assert_eq!(ModelCategoryName::parse(name.as_str()), Some(*name));
    }
    assert_eq!(ModelCategoryName::parse("Fast"), None);
    assert_eq!(ModelCategoryName::parse("nope"), None);
    assert_eq!(
        serde_json::to_value(Cheap).unwrap(),
        json!("cheap"),
        "serde uses lowercase names"
    );
}

#[test]
fn scoped_model_serializes_camel_case_and_skips_none() {
    let plain = entry("openai/gpt-5");
    assert_eq!(
        serde_json::to_value(&plain).unwrap(),
        json!({"model": "openai/gpt-5"})
    );
    let full = ScopedModel {
        model: "anthropic/claude-opus".into(),
        effort: Some("high".into()),
        category: Some(ModelCategoryName::Capable),
        alias: Some("opus".into()),
    };
    assert_eq!(
        serde_json::to_value(&full).unwrap(),
        json!({
            "model": "anthropic/claude-opus",
            "effort": "high",
            "category": "capable",
            "alias": "opus"
        })
    );
    let back: ScopedModel = serde_json::from_value(serde_json::to_value(&full).unwrap()).unwrap();
    assert_eq!(back, full);
}

#[test]
fn migrates_enabled_models_and_categories_with_level_suffix_as_effort() {
    let dirs = Dirs::new();
    dirs.write_global(json!({
        "enabledModels": ["anthropic/claude-opus:high", "openai/gpt-5", "openrouter/foo:free"],
        "modelCategories": {
            "fast": "openai/gpt-5-mini",
            "capable": "anthropic/claude-opus"
        }
    }));
    let manager = dirs.manager();
    assert_eq!(
        manager.scoped_models().unwrap(),
        vec![
            ScopedModel {
                model: "anthropic/claude-opus".into(),
                effort: Some("high".into()),
                category: Some(ModelCategoryName::Capable),
                alias: None,
            },
            entry("openai/gpt-5"),
            // A colon that is not a thinking level stays part of the id.
            entry("openrouter/foo:free"),
            ScopedModel {
                model: "openai/gpt-5-mini".into(),
                effort: None,
                category: Some(ModelCategoryName::Fast),
                alias: None,
            },
        ]
    );
}

#[test]
fn migrates_from_categories_only_in_tier_order() {
    let dirs = Dirs::new();
    dirs.write_global(json!({
        "modelCategories": {"capable": "b/capable", "standard": "b/standard", "fast": "b/fast"}
    }));
    let models = dirs.manager().scoped_models().unwrap();
    let got: Vec<(&str, Option<ModelCategoryName>)> = models
        .iter()
        .map(|m| (m.model.as_str(), m.category))
        .collect();
    assert_eq!(
        got,
        vec![
            ("b/fast", Some(ModelCategoryName::Fast)),
            ("b/standard", Some(ModelCategoryName::Standard)),
            ("b/capable", Some(ModelCategoryName::Capable)),
        ]
    );
}

#[test]
fn migrates_enabled_models_alone_without_categories() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"enabledModels": ["a/*", "b/x:low"]}));
    assert_eq!(
        dirs.manager().scoped_models().unwrap(),
        vec![
            entry("a/*"),
            ScopedModel {
                model: "b/x".into(),
                effort: Some("low".into()),
                category: None,
                alias: None,
            },
        ]
    );
}

#[test]
fn neither_legacy_key_writes_nothing() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"theme": "dark"}));
    let manager = dirs.manager();
    assert_eq!(manager.scoped_models(), None);
    assert!(dirs.read_global().get("scopedModels").is_none());
}

#[test]
fn present_empty_list_counts_as_present_and_is_not_migrated() {
    let dirs = Dirs::new();
    dirs.write_global(json!({
        "scopedModels": [],
        "enabledModels": ["openai/gpt-5"],
        "modelCategories": {"fast": "openai/gpt-5-mini"}
    }));
    assert_eq!(dirs.manager().scoped_models(), Some(vec![]));
    assert_eq!(dirs.read_global()["scopedModels"], json!([]));
}

#[test]
fn migration_is_written_once_and_second_load_does_not_repeat_it() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"enabledModels": ["openai/gpt-5"]}));
    let first = dirs.manager();
    assert_eq!(first.scoped_models().unwrap(), vec![entry("openai/gpt-5")]);
    assert_eq!(
        dirs.read_global()["scopedModels"],
        json!([{"model": "openai/gpt-5"}]),
        "migrated key is on disk after first load"
    );

    // Edit the retired key on disk. Migration already ran, so it is ignored.
    let mut on_disk = dirs.read_global();
    on_disk["enabledModels"] = json!(["anthropic/claude-x"]);
    write_json(&dirs.global_path(), &on_disk);

    let second = dirs.manager();
    assert_eq!(second.scoped_models().unwrap(), vec![entry("openai/gpt-5")]);
    assert_eq!(
        dirs.read_global()["scopedModels"],
        json!([{"model": "openai/gpt-5"}])
    );
}

#[test]
fn old_keys_stay_on_disk_and_are_not_read_after_migration() {
    let dirs = Dirs::new();
    dirs.write_global(json!({
        "enabledModels": ["openai/gpt-5"],
        "modelCategories": {"fast": "openai/gpt-5-mini"}
    }));
    let manager = dirs.manager();
    assert_eq!(manager.scoped_models().unwrap().len(), 2);
    let on_disk = dirs.read_global();
    assert!(on_disk.get("enabledModels").is_some());
    assert!(on_disk.get("modelCategories").is_some());
}

#[test]
fn in_memory_manager_migrates_too() {
    let mut settings = serde_json::Map::new();
    settings.insert("enabledModels".into(), json!(["openai/gpt-5:medium"]));
    let manager = SettingsManager::in_memory(settings);
    assert_eq!(
        manager.scoped_models().unwrap(),
        vec![ScopedModel {
            model: "openai/gpt-5".into(),
            effort: Some("medium".into()),
            category: None,
            alias: None,
        }]
    );
}

#[test]
fn set_scoped_models_round_trips_through_disk() {
    let dirs = Dirs::new();
    let list = vec![
        ScopedModel {
            model: "anthropic/claude-haiku".into(),
            effort: Some("off".into()),
            category: Some(ModelCategoryName::Cheap),
            alias: Some("haiku".into()),
        },
        entry("openai/gpt-5"),
    ];
    let mut manager = dirs.manager();
    manager.set_scoped_models(&list);
    assert_eq!(manager.scoped_models().unwrap(), list);
    assert_eq!(
        dirs.read_global()["scopedModels"],
        json!([
            {"model": "anthropic/claude-haiku", "effort": "off", "category": "cheap", "alias": "haiku"},
            {"model": "openai/gpt-5"}
        ])
    );
    assert_eq!(dirs.manager().scoped_models().unwrap(), list);
}

#[test]
fn project_scoped_models_replace_global_and_do_not_merge() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"scopedModels": [
        {"model": "a/one"}, {"model": "a/two", "category": "fast"}
    ]}));
    write_json(
        &dirs.project_path(),
        &json!({"scopedModels": [{"model": "b/three", "category": "capable"}]}),
    );
    let merged = dirs.manager().scoped_models().unwrap();
    assert_eq!(
        merged,
        vec![ScopedModel {
            model: "b/three".into(),
            effort: None,
            category: Some(ModelCategoryName::Capable),
            alias: None,
        }]
    );
}

#[test]
fn project_without_scoped_models_keeps_global_list() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"scopedModels": [{"model": "a/one"}]}));
    write_json(&dirs.project_path(), &json!({"theme": "light"}));
    assert_eq!(
        dirs.manager().scoped_models().unwrap(),
        vec![entry("a/one")]
    );
}

#[test]
fn project_scoped_models_are_not_migrated_from_project_legacy_keys() {
    let dirs = Dirs::new();
    write_json(
        &dirs.project_path(),
        &json!({"enabledModels": ["openai/gpt-5"]}),
    );
    let manager = dirs.manager();
    assert_eq!(manager.scoped_models(), None);
    assert!(dirs.read_global().get("scopedModels").is_none());
}

#[test]
fn validate_accepts_unique_lowercase_aliases() {
    let list = vec![
        ScopedModel {
            alias: Some("fast-1".into()),
            ..entry("a/x")
        },
        ScopedModel {
            alias: Some("opus2".into()),
            ..entry("a/y")
        },
        entry("a/z"),
    ];
    assert_eq!(SettingsManager::validate_scoped_models(&list), Ok(()));
    assert_eq!(SettingsManager::validate_scoped_models(&[]), Ok(()));
}

#[test]
fn validate_rejects_duplicate_aliases() {
    let list = vec![
        ScopedModel {
            alias: Some("opus".into()),
            ..entry("a/x")
        },
        ScopedModel {
            alias: Some("opus".into()),
            ..entry("a/y")
        },
    ];
    let err = SettingsManager::validate_scoped_models(&list).unwrap_err();
    assert!(err.contains("opus"), "{err}");
}

#[test]
fn validate_rejects_aliases_outside_a_to_z_0_to_9_dash() {
    for bad in ["Opus", "my_model", "has space", "dot.ted", "", "é"] {
        let list = vec![ScopedModel {
            alias: Some(bad.into()),
            ..entry("a/x")
        }];
        assert!(
            SettingsManager::validate_scoped_models(&list).is_err(),
            "alias {bad:?} should be rejected"
        );
    }
}

#[test]
fn validate_rejects_an_empty_model() {
    assert!(SettingsManager::validate_scoped_models(&[entry("  ")]).is_err());
}

#[test]
fn set_scoped_models_writes_the_project_file_when_it_defines_the_key() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"scopedModels": [{"model": "a/global"}]}));
    write_json(
        &dirs.project_path(),
        &json!({"scopedModels": [{"model": "a/project"}]}),
    );
    let mut manager = dirs.manager();
    manager.set_scoped_models(&[entry("a/saved")]);
    assert_eq!(
        dirs.read_global()["scopedModels"],
        json!([{"model": "a/global"}])
    );
    let project: Value =
        serde_json::from_str(&std::fs::read_to_string(dirs.project_path()).unwrap()).unwrap();
    assert_eq!(project["scopedModels"], json!([{"model": "a/saved"}]));
    assert_eq!(
        dirs.manager().scoped_models().unwrap(),
        vec![entry("a/saved")]
    );
}

#[test]
fn set_scoped_models_writes_global_when_the_project_has_no_key() {
    let dirs = Dirs::new();
    write_json(&dirs.project_path(), &json!({"model": "x"}));
    let mut manager = dirs.manager();
    manager.set_scoped_models(&[entry("a/saved")]);
    assert_eq!(
        dirs.read_global()["scopedModels"],
        json!([{"model": "a/saved"}])
    );
    assert!(!std::fs::read_to_string(dirs.project_path())
        .unwrap()
        .contains("scopedModels"));
}

#[test]
fn clearing_with_an_empty_slice_keeps_the_key_so_no_migration_runs() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"enabledModels": ["a/old"]}));
    let mut manager = dirs.manager();
    manager.set_scoped_models(&[]);
    assert_eq!(dirs.read_global()["scopedModels"], json!([]));
    assert_eq!(dirs.manager().scoped_models().unwrap(), Vec::new());
}

#[test]
fn validate_rejects_an_alias_on_a_glob_entry() {
    let list = vec![ScopedModel {
        alias: Some("anth".into()),
        ..entry("anthropic/*")
    }];
    let err = SettingsManager::validate_scoped_models(&list).unwrap_err();
    assert!(err.contains("glob"), "{err}");
}

#[test]
fn validate_rejects_aliases_that_name_a_category() {
    for tier in ["cheap", "fast", "standard", "capable"] {
        let list = vec![ScopedModel {
            alias: Some(tier.into()),
            ..entry("a/x")
        }];
        assert!(
            SettingsManager::validate_scoped_models(&list).is_err(),
            "alias {tier} should be rejected"
        );
    }
}

#[test]
fn an_unparseable_scoped_model_entry_is_skipped_and_reported() {
    let dirs = Dirs::new();
    dirs.write_global(json!({"scopedModels": [
        {"model": "a/good"},
        {"model": "a/bad", "category": "premium"},
        {"model": "a/also-good", "effort": "high"}
    ]}));
    let mut manager = dirs.manager();
    assert_eq!(
        manager.scoped_models().unwrap(),
        vec![
            entry("a/good"),
            ScopedModel {
                effort: Some("high".into()),
                ..entry("a/also-good")
            }
        ]
    );
    let errors = manager.drain_errors();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(matches!(
        &errors[0].error,
        Error::InvalidScopedModel(message) if message.contains("scopedModels[1]")
    ));
}

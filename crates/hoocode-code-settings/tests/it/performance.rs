//! The four `performance.*` settings: defaults and range clamping.

use hoocode_code_settings::SettingsManager;
use serde_json::{json, Value};

fn manager(performance: Value) -> SettingsManager {
    SettingsManager::in_memory(
        json!({ "performance": performance })
            .as_object()
            .unwrap()
            .clone(),
    )
}

#[test]
fn defaults_when_unset() {
    let m = SettingsManager::in_memory(Default::default());
    assert_eq!(m.performance_max_parallel_tools(), 8);
    assert_eq!(m.performance_bash_nice(), 0);

    let total = hoocode_runtime::total_memory_bytes();
    let mib = 1024 * 1024;
    let (soft, hard) = match total {
        Some(bytes) => (
            (bytes / mib / 4).clamp(256, 2048),
            (bytes / mib / 2).clamp(256, 4096),
        ),
        None => (2048, 4096),
    };
    assert_eq!(m.performance_memory_soft_limit_mb(), soft);
    assert_eq!(m.performance_memory_hard_limit_mb(), hard.max(soft + 1));
    assert!(m.performance_memory_hard_limit_mb() > m.performance_memory_soft_limit_mb());
}

#[test]
fn max_parallel_tools_is_clamped_to_one_through_thirty_two() {
    assert_eq!(
        manager(json!({"maxParallelTools": 0})).performance_max_parallel_tools(),
        1
    );
    assert_eq!(
        manager(json!({"maxParallelTools": -4})).performance_max_parallel_tools(),
        1
    );
    assert_eq!(
        manager(json!({"maxParallelTools": 5.9})).performance_max_parallel_tools(),
        5
    );
    assert_eq!(
        manager(json!({"maxParallelTools": 32})).performance_max_parallel_tools(),
        32
    );
    assert_eq!(
        manager(json!({"maxParallelTools": 500})).performance_max_parallel_tools(),
        32
    );
    assert_eq!(
        manager(json!({"maxParallelTools": "many"})).performance_max_parallel_tools(),
        8
    );
}

#[test]
fn bash_nice_is_clamped_to_zero_through_nineteen() {
    assert_eq!(manager(json!({"bashNice": -5})).performance_bash_nice(), 0);
    assert_eq!(manager(json!({"bashNice": 10})).performance_bash_nice(), 10);
    assert_eq!(manager(json!({"bashNice": 25})).performance_bash_nice(), 19);
}

#[test]
fn memory_soft_limit_zero_is_off_and_small_values_rise_to_256() {
    assert_eq!(
        manager(json!({"memorySoftLimitMb": 0})).performance_memory_soft_limit_mb(),
        0
    );
    assert_eq!(
        manager(json!({"memorySoftLimitMb": 100})).performance_memory_soft_limit_mb(),
        256
    );
    assert_eq!(
        manager(json!({"memorySoftLimitMb": 1024})).performance_memory_soft_limit_mb(),
        1024
    );
}

#[test]
fn memory_hard_limit_stays_above_the_soft_limit() {
    let m = manager(json!({"memorySoftLimitMb": 1024, "memoryHardLimitMb": 1000}));
    assert_eq!(m.performance_memory_hard_limit_mb(), 1025);

    let m = manager(json!({"memorySoftLimitMb": 1024, "memoryHardLimitMb": 8000}));
    assert_eq!(m.performance_memory_hard_limit_mb(), 8000);

    let m = manager(json!({"memorySoftLimitMb": 1024, "memoryHardLimitMb": 0}));
    assert_eq!(m.performance_memory_hard_limit_mb(), 0);
}

#[test]
fn memory_hard_limit_with_soft_off_has_the_256_floor() {
    let m = manager(json!({"memorySoftLimitMb": 0, "memoryHardLimitMb": 100}));
    assert_eq!(m.performance_memory_hard_limit_mb(), 256);
}

#[test]
fn non_object_performance_block_uses_defaults() {
    let m = manager(json!("oops"));
    assert_eq!(m.performance_max_parallel_tools(), 8);
    assert_eq!(m.performance_bash_nice(), 0);
}

use rand::Rng;
use serde_json::Value;
use std::collections::HashMap;

use super::account::{CanonicalEnvData, CanonicalProcessData, CanonicalPromptEnvData};

pub const CLAUDE_CODE_VERSION: &str = "2.1.280";
pub const CLAUDE_CODE_BUILD_TIME: &str = "2026-09-21T20:40:17Z";
pub const CLAUDE_CODE_STAINLESS_VERSION: &str = "0.112.1";
pub const CLAUDE_CODE_RUNTIME_VERSION: &str = "v26.3.0";

fn env_presets() -> Vec<CanonicalEnvData> {
    vec![
        // --- darwin arm64 (8 presets) ---
        dp(
            "arm64",
            CLAUDE_CODE_RUNTIME_VERSION,
            "iTerm.app",
            "npm,pnpm",
        ),
        dp(
            "arm64",
            CLAUDE_CODE_RUNTIME_VERSION,
            "Apple_Terminal",
            "npm,yarn",
        ),
        dp("arm64", CLAUDE_CODE_RUNTIME_VERSION, "vscode", "npm,pnpm"),
        dp("arm64", CLAUDE_CODE_RUNTIME_VERSION, "WarpTerminal", "npm"),
        dp(
            "arm64",
            CLAUDE_CODE_RUNTIME_VERSION,
            "kitty",
            "npm,yarn,pnpm",
        ),
        dp("arm64", CLAUDE_CODE_RUNTIME_VERSION, "iTerm.app", "npm"),
        dp("arm64", CLAUDE_CODE_RUNTIME_VERSION, "tmux", "npm,pnpm"),
        dp("arm64", CLAUDE_CODE_RUNTIME_VERSION, "ghostty", "npm,yarn"),
        // --- darwin x64 (4 presets) ---
        dx(CLAUDE_CODE_RUNTIME_VERSION, "iTerm.app", "npm,yarn"),
        dx(CLAUDE_CODE_RUNTIME_VERSION, "Apple_Terminal", "npm,pnpm"),
        dx(CLAUDE_CODE_RUNTIME_VERSION, "vscode", "npm"),
        dx(CLAUDE_CODE_RUNTIME_VERSION, "iTerm.app", "npm,pnpm"),
        // --- linux (6 presets) ---
        lp(CLAUDE_CODE_RUNTIME_VERSION, "gnome-terminal", "npm,pnpm"),
        lp(CLAUDE_CODE_RUNTIME_VERSION, "ssh-session", "npm"),
        lp(CLAUDE_CODE_RUNTIME_VERSION, "xterm-256color", "npm,yarn"),
        lp(CLAUDE_CODE_RUNTIME_VERSION, "vscode", "npm,pnpm"),
        lp(CLAUDE_CODE_RUNTIME_VERSION, "tmux", "npm"),
        lp(CLAUDE_CODE_RUNTIME_VERSION, "alacritty", "npm,yarn"),
        // --- win32 (4 presets) ---
        wp(CLAUDE_CODE_RUNTIME_VERSION, "windows-terminal", "npm,pnpm"),
        wp(CLAUDE_CODE_RUNTIME_VERSION, "vscode", "npm,yarn"),
        wp(CLAUDE_CODE_RUNTIME_VERSION, "mingw64", "npm"),
        wp(CLAUDE_CODE_RUNTIME_VERSION, "windows-terminal", "npm,pnpm"),
    ]
}

fn dp(arch: &str, node: &str, term: &str, pm: &str) -> CanonicalEnvData {
    CanonicalEnvData {
        platform: "darwin".into(),
        platform_raw: "darwin".into(),
        arch: arch.into(),
        node_version: node.into(),
        terminal: term.into(),
        shell: "zsh".into(),
        package_managers: pm.into(),
        runtimes: "node".into(),
        is_running_with_bun: true,
        is_claude_ai_auth: true,
        version: CLAUDE_CODE_VERSION.into(),
        version_base: CLAUDE_CODE_VERSION.into(),
        build_time: CLAUDE_CODE_BUILD_TIME.into(),
        deployment_environment: "unknown-darwin".into(),
        vcs: "git".into(),
        ..Default::default()
    }
}

fn dx(node: &str, term: &str, pm: &str) -> CanonicalEnvData {
    dp("x64", node, term, pm)
}

fn lp(node: &str, term: &str, pm: &str) -> CanonicalEnvData {
    CanonicalEnvData {
        platform: "linux".into(),
        platform_raw: "linux".into(),
        arch: "x64".into(),
        node_version: node.into(),
        terminal: term.into(),
        shell: "bash".into(),
        package_managers: pm.into(),
        runtimes: "node".into(),
        is_running_with_bun: true,
        is_claude_ai_auth: true,
        version: CLAUDE_CODE_VERSION.into(),
        version_base: CLAUDE_CODE_VERSION.into(),
        build_time: CLAUDE_CODE_BUILD_TIME.into(),
        deployment_environment: "unknown-linux".into(),
        vcs: "git".into(),
        ..Default::default()
    }
}

fn wp(node: &str, term: &str, pm: &str) -> CanonicalEnvData {
    CanonicalEnvData {
        platform: "win32".into(),
        platform_raw: "win32".into(),
        arch: "x64".into(),
        node_version: node.into(),
        terminal: term.into(),
        shell: "bash".into(),
        package_managers: pm.into(),
        runtimes: "node".into(),
        is_running_with_bun: true,
        is_claude_ai_auth: true,
        version: CLAUDE_CODE_VERSION.into(),
        version_base: CLAUDE_CODE_VERSION.into(),
        build_time: CLAUDE_CODE_BUILD_TIME.into(),
        deployment_environment: "unknown-win32".into(),
        vcs: "git".into(),
        ..Default::default()
    }
}

fn prompt_presets() -> HashMap<&'static str, CanonicalPromptEnvData> {
    let mut m = HashMap::new();
    m.insert(
        "darwin",
        CanonicalPromptEnvData {
            platform: "darwin".into(),
            shell: "zsh".into(),
            os_version: "Darwin 24.4.0".into(),
            working_dir: "/Users/user/projects".into(),
        },
    );
    m.insert(
        "linux",
        CanonicalPromptEnvData {
            platform: "linux".into(),
            shell: "bash".into(),
            os_version: "Linux 6.5.0-generic".into(),
            working_dir: "/home/user/projects".into(),
        },
    );
    m.insert(
        "win32",
        CanonicalPromptEnvData {
            platform: "win32".into(),
            shell: "bash (use Unix shell syntax, not Windows \u{2014} e.g., /dev/null not NUL, forward slashes in paths)".into(),
            os_version: "Windows 10 Pro 10.0.19045".into(),
            working_dir: "/c/Users/user/projects".into(),
        },
    );
    m
}

static MEMORY_PRESETS: &[i64] = &[
    0, // process.constrainedMemory() returns 0 on non-containerized environments
];

/// 生成随机的 64 字符十六进制字符串。
pub fn generate_device_id() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill(&mut bytes);
    hex::encode(bytes)
}

/// 为新账号生成全部规范化身份字段。
pub fn generate_canonical_identity() -> (String, Value, Value, Value) {
    let device_id = generate_device_id();
    let mut rng = rand::thread_rng();

    let presets = env_presets();
    let preset = &presets[rng.gen_range(0..presets.len())];
    let env_json = serde_json::to_value(preset).expect("env preset serialize");

    let prompts = prompt_presets();
    let prompt_env = prompts
        .get(preset.platform.as_str())
        .expect("prompt preset");
    let prompt_json = serde_json::to_value(prompt_env).expect("prompt preset serialize");

    let mem = MEMORY_PRESETS[rng.gen_range(0..MEMORY_PRESETS.len())];
    let process = CanonicalProcessData {
        constrained_memory: mem,
        rss_range: [300_000_000, 500_000_000],
        heap_total_range: [100_000_000, 200_000_000],
        heap_used_range: [40_000_000, 80_000_000],
        external_range: [1_000_000, 3_000_000],
        array_buffers_range: [10_000, 50_000],
    };
    let process_json = serde_json::to_value(&process).expect("process serialize");

    (device_id, env_json, prompt_json, process_json)
}

/// 将持久化环境中的版本绑定字段规范化到当前客户端 release profile。
/// 直接修改 JSON object，保留未来版本或手工写入的未知字段。
pub fn normalize_canonical_env_json(value: &mut Value) {
    if !value.is_object() {
        *value = Value::Object(Default::default());
    }
    let map = value.as_object_mut().expect("canonical env object");
    map.insert("version".into(), Value::String(CLAUDE_CODE_VERSION.into()));
    map.insert(
        "version_base".into(),
        Value::String(CLAUDE_CODE_VERSION.into()),
    );
    map.insert(
        "build_time".into(),
        Value::String(CLAUDE_CODE_BUILD_TIME.into()),
    );
    map.insert(
        "node_version".into(),
        Value::String(CLAUDE_CODE_RUNTIME_VERSION.into()),
    );
    map.insert("runtimes".into(), Value::String("node".into()));
    map.insert("is_running_with_bun".into(), Value::Bool(true));

    let shell = match map.get("platform").and_then(Value::as_str) {
        Some("darwin") => "zsh",
        Some("linux" | "win32") => "bash",
        _ => "",
    };
    if !shell.is_empty()
        && !map
            .get("shell")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        map.insert("shell".into(), Value::String(shell.into()));
    }
}

pub fn parse_canonical_env(value: &Value) -> CanonicalEnvData {
    let mut normalized = value.clone();
    normalize_canonical_env_json(&mut normalized);
    serde_json::from_value(normalized).unwrap_or_default()
}

/// 构造 proto schema 完整的 env JSON（含所有 ~30 个字段）。
/// 供 rewriter 和 telemetry 共用，避免重复定义。
pub fn build_full_env_json(env: &CanonicalEnvData) -> Value {
    let mut value = serde_json::json!({
        "platform": env.platform,
        "platform_raw": env.platform_raw,
        "arch": env.arch,
        "node_version": env.node_version,
        "terminal": if env.terminal.is_empty() { "unknown" } else { &env.terminal },
        "shell": env.shell,
        "package_managers": env.package_managers,
        "runtimes": env.runtimes,
        "is_running_with_bun": env.is_running_with_bun,
        "is_ci": env.is_ci,
        "is_claubbit": env.is_claubbit,
        "is_claude_code_remote": env.is_claude_code_remote,
        "is_local_agent_mode": env.is_local_agent_mode,
        "is_conductor": env.is_conductor,
        "is_github_action": env.is_github_action,
        "is_claude_code_action": env.is_claude_code_action,
        "is_claude_ai_auth": env.is_claude_ai_auth,
        "version": env.version,
        "version_base": env.version_base,
        "build_time": env.build_time,
        "deployment_environment": env.deployment_environment,
        "vcs": env.vcs,
    });
    let map = value.as_object_mut().expect("full env object");
    map.retain(|_, item| !matches!(item, Value::String(text) if text.is_empty()));
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_updates_release_fields_and_preserves_identity_fields() {
        let mut value = serde_json::json!({
            "platform": "darwin",
            "arch": "arm64",
            "terminal": "iTerm.app",
            "package_managers": "npm,pnpm",
            "version": "2.1.81",
            "build_time": "old",
            "future_field": {"enabled": true}
        });
        normalize_canonical_env_json(&mut value);
        assert_eq!(value["version"], CLAUDE_CODE_VERSION);
        assert_eq!(value["version_base"], CLAUDE_CODE_VERSION);
        assert_eq!(value["build_time"], CLAUDE_CODE_BUILD_TIME);
        assert_eq!(value["node_version"], CLAUDE_CODE_RUNTIME_VERSION);
        assert_eq!(value["terminal"], "iTerm.app");
        assert_eq!(value["package_managers"], "npm,pnpm");
        assert_eq!(value["future_field"]["enabled"], true);
    }

    #[test]
    fn full_env_uses_bun_shell_and_omits_empty_optional_values() {
        let env = CanonicalEnvData {
            platform: "darwin".into(),
            platform_raw: "darwin".into(),
            arch: "arm64".into(),
            node_version: CLAUDE_CODE_RUNTIME_VERSION.into(),
            terminal: "iTerm.app".into(),
            shell: "zsh".into(),
            package_managers: "npm".into(),
            runtimes: "node".into(),
            is_running_with_bun: true,
            version: CLAUDE_CODE_VERSION.into(),
            version_base: CLAUDE_CODE_VERSION.into(),
            build_time: CLAUDE_CODE_BUILD_TIME.into(),
            ..Default::default()
        };
        let value = build_full_env_json(&env);
        assert_eq!(value["is_running_with_bun"], true);
        assert_eq!(value["shell"], "zsh");
        assert!(value.get("remote_environment_type").is_none());
    }
}

//! 编译时自动生成的版本信息
//!
//! 由 build.rs 通过 built 生成，包含 Git 提交、构建时间等。

// 引入 built 生成的代码（定义了 PKG_VERSION、GIT_COMMIT_HASH 等常量）
include!(concat!(env!("OUT_DIR"), "/built.rs"));

/// 程序版本号（来自 Cargo.toml）
pub const APP_VERSION: &str = PKG_VERSION;

/// Git 提交哈希（短，前 7 位）
pub fn git_commit_short() -> &'static str {
    match GIT_COMMIT_HASH_SHORT {
        Some(hash) => hash,
        None => "unknown",
    }
}

/// Git 分支名（从 GIT_HEAD_REF 提取，如 refs/heads/master → master）
pub fn git_branch() -> &'static str {
    match GIT_HEAD_REF {
        Some(head_ref) => {
            // 提取分支名，如 "refs/heads/master" → "master"
            head_ref.strip_prefix("refs/heads/").unwrap_or(head_ref)
        }
        None => "unknown",
    }
}

/// 构建时间
pub const APP_BUILD_TIME: &str = BUILT_TIME_UTC;

/// 构建目标平台
pub const APP_TARGET: &str = TARGET;

/// 综合版本信息字符串
pub fn full_version() -> String {
    format!(
        "{} ({} {})",
        APP_VERSION,
        git_commit_short(),
        git_branch()
    )
}

/// 构建信息字符串
pub fn build_info() -> String {
    format!(
        "构建时间: {}\n目标平台: {}\nGit 提交: {}",
        APP_BUILD_TIME,
        APP_TARGET,
        git_commit_short()
    )
}
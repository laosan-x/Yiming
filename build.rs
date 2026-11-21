use std::env;

fn main() {
    // ── 1. 自动生成版本信息（由 built 生成到 OUT_DIR/built.rs） ──
    built::write_built_file().expect("Failed to generate built.rs");

    // ── 2. Windows EXE 图标嵌入（仅 release）──
    // winres 需要查询注册表定位 Windows SDK 的 rc.exe；dev/test 构建不需要
    // exe 图标资源，跳过可避免受限环境（无注册表访问）下编译失败。
    if env::var("CARGO_CFG_WINDOWS").is_ok()
        && env::var("PROFILE").as_deref() == Ok("release")
    {
        let mut res = winres::WindowsResource::new();
        // 受限环境（无注册表访问，如 CI 沙箱）下 winres 的 get_sdk() 会调 reg.exe
        // 失败并回退到空路径，导致 compile() 找不到 rc.exe。允许用环境变量显式
        // 指定含 rc.exe 的目录（如 .../Windows Kits/10/bin/10.0.26100.0/x64），
        // 绕过注册表查询；未设置时行为与原来完全一致（CI 正常构建不受影响）。
        if let Ok(toolkit) = env::var("COCOLE_RC_TOOLKIT") {
            if !toolkit.is_empty() {
                res.set_toolkit_path(&toolkit);
                // 图标资源不涉及 Windows SDK 头文件；winres 默认
                // add_toolkit_include=false，不会调 win_sdk_inlcude_root() 查注册表。
            }
        }
        res.set_icon("icons/app.ico");
        res.compile().expect("Failed to compile Windows resource");
    }
}

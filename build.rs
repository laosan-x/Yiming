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
        res.set_icon("icons/app.ico");
        res.compile().expect("Failed to compile Windows resource");
    }
}

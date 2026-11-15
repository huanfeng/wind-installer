fn main() {
    // 静态 CRT（crt-static）+ 调试信息时，lld-link 找不到微软 libcmt.lib 对应的 pdb，
    // 每个 CRT 对象一条 LNK4099，刷屏且无可操作项；只是缺调试符号，不影响产物，故屏蔽。
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/IGNORE:4099");
    }

    // 嵌入 UAC manifest + 默认图标到两个 GUI 二进制。
    // 应用身份不再来自 Cargo.toml——已改为打包时由 app.toml 嵌入归档（见 src/manifest.rs）。
    // 安装器 EXE 的最终图标由 wind-packer 用 editpe 按 app.toml 覆盖；此处仅提供默认占位图标。
    embed_resource::compile_for(
        "assets/app.rc",
        ["wind-installer", "wind-uninstaller"],
        embed_resource::NONE,
    );
}

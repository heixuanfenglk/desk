fn main() {
    println!("cargo:rerun-if-changed=assets/desk.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/desk.ico");
        res.compile().expect("嵌入程序图标");
    }
}

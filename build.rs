//! Gives the executable its icon and version information.

fn main() {
    println!("cargo:rerun-if-changed=assets/mocli.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut resource = winresource::WindowsResource::new();
    resource
        .set_icon("assets/mocli.ico")
        .set("ProductName", "mocli")
        .set("FileDescription", "mocli")
        .set("CompanyName", "bennett-sh");
    if let Err(error) = resource.compile() {
        panic!("could not compile the Windows resources: {error}");
    }
}

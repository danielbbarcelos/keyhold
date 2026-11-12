//! Windows only: stamp the executables with who made them, what they are and their version.
//!
//! An executable with no version information at all is one of the things antivirus heuristics
//! treat as suspicious, and it is what Windows shows in the file's properties.

fn main() {
    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
            let mut res = winresource::WindowsResource::new();
            res.set_icon("packaging/windows/keyhold.ico");
            res.set("ProductName", "Keyhold");
            res.set("FileDescription", "Keyhold, a local password vault");
            res.set("CompanyName", "Keyhold");
            res.set("LegalCopyright", "MIT License");
            res.set("OriginalFilename", "keyhold.exe");
            res.compile().expect("the Windows resources did not compile");
        }
    }
}

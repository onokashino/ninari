fn main() {
    // The program's icon on Windows: GPUI gives its windows icon resource 1, Explorer shows it too.
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/icon.ico");
        res.compile().expect("the Windows icon resource");
    }
}

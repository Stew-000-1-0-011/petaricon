fn main() {
    // ComponentContainer / component-factory are still experimental in Slint 1.18.
    // SAFETY: build scripts are single-threaded at this point.
    unsafe { std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1") };
    slint_build::compile("ui/editor.slint").unwrap();
}

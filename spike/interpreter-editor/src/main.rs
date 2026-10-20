// Desktop entry point, used to iterate before building the APK.
fn main() -> Result<(), slint::PlatformError> {
    interp_editor_spike::run()
}

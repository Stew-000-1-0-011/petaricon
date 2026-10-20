// Desktop entry point, used to iterate quickly before building the APK.
fn main() -> Result<(), slint::PlatformError> {
    slint::BackendSelector::new()
        .require_wgpu_30(slint::wgpu_30::WGPUConfiguration::default())
        .select()?;
    pointcloud_spike::run()
}

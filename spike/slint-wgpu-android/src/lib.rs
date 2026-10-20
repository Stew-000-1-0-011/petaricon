//! Throwaway spike: can a Slint Android app embed a wgpu-rendered 3D point cloud?

mod cloud;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use slint::ComponentHandle;

slint::include_modules!();

const INITIAL_POINTS: u32 = 100_000;

struct State {
    renderer: Option<cloud::CloudRenderer>,
    point_count: u32,
    pending_count: Option<u32>,
    last_frame: Instant,
    fps_window_start: Instant,
    frames: u32,
    fps: f32,
    gpu_ms: f32,
}

pub fn run() -> Result<(), slint::PlatformError> {
    let app = AppWindow::new()?;
    let state = Rc::new(RefCell::new(State {
        renderer: None,
        point_count: INITIAL_POINTS,
        pending_count: None,
        last_frame: Instant::now(),
        fps_window_start: Instant::now(),
        frames: 0,
        fps: 0.0,
        gpu_ms: 0.0,
    }));

    {
        let state = state.clone();
        app.on_set_point_count(move |n| {
            state.borrow_mut().pending_count = Some(n as u32);
        });
    }

    let app_weak = app.as_weak();
    app.window()
        .set_rendering_notifier(move |rendering_state, graphics_api| {
            let Some(app) = app_weak.upgrade() else { return };
            let mut st = state.borrow_mut();
            match rendering_state {
                slint::RenderingState::RenderingSetup => {
                    if let slint::GraphicsAPI::WGPU30 { device, queue, .. } = graphics_api {
                        let mut r = cloud::CloudRenderer::new(device, queue);
                        r.set_points(st.point_count);
                        st.renderer = Some(r);
                    } else {
                        app.set_stats_text(format!("NOT wgpu: {graphics_api:?}").into());
                    }
                }
                slint::RenderingState::BeforeRendering => {
                    if st.renderer.is_none() {
                        return;
                    }
                    if let Some(n) = st.pending_count.take() {
                        st.point_count = n;
                        st.renderer.as_mut().unwrap().set_points(n);
                    }

                    let now = Instant::now();
                    let dt = (now - st.last_frame).as_secs_f32().min(0.1);
                    st.last_frame = now;
                    app.set_yaw(app.get_yaw() + dt * app.get_spin_speed());

                    let scale = app.window().scale_factor();
                    let w = (app.get_view_width() * scale) as u32;
                    let h = (app.get_view_height() * scale) as u32;
                    let t0 = Instant::now();
                    let tex = st.renderer.as_mut().unwrap().render(
                        w,
                        h,
                        app.get_yaw(),
                        app.get_pitch(),
                        app.get_zoom(),
                    );
                    st.gpu_ms = st.gpu_ms * 0.9 + t0.elapsed().as_secs_f32() * 1000.0 * 0.1;
                    match slint::Image::try_from(tex) {
                        Ok(img) => app.set_cloud_texture(img),
                        Err(e) => app.set_stats_text(format!("texture import failed: {e}").into()),
                    }

                    st.frames += 1;
                    let elapsed = (now - st.fps_window_start).as_secs_f32();
                    if elapsed >= 1.0 {
                        st.fps = st.frames as f32 / elapsed;
                        st.frames = 0;
                        st.fps_window_start = now;
                        app.set_stats_text(
                            format!(
                                "{:.1} fps | {} pts | {}x{} px | cpu {:.2} ms/frame",
                                st.fps, st.point_count, w, h, st.gpu_ms
                            )
                            .into(),
                        );
                    }
                    // Keep animating.
                    app.window().request_redraw();
                }
                slint::RenderingState::RenderingTeardown => {
                    st.renderer = None;
                }
                _ => {}
            }
        })
        .map_err(|e| slint::PlatformError::Other(format!("rendering notifier: {e:?}")))?;

    app.run()
}

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(android_app: slint::android::AndroidApp) {
    slint::android::init(android_app).unwrap();
    run().unwrap();
}

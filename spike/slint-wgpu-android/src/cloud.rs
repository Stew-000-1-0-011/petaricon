//! Point cloud renderer: draws instanced screen-space quads into an
//! offscreen RGBA texture that Slint then shows as an `Image`.

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use slint::wgpu_30::wgpu;

const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Point {
    pos: [f32; 3],
    color: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniforms {
    view_proj: [[f32; 4]; 4],
    // x, y: point half-size in NDC units, z/w unused.
    point_size: [f32; 4],
}

const SHADER: &str = r#"
struct Uniforms {
    view_proj: mat4x4<f32>,
    point_size: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32,
           @location(0) center: vec3<f32>,
           @location(1) color: vec4<f32>) -> VsOut {
    var corners = array<vec2<f32>, 6>(
        vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(1.0, 1.0),
        vec2(-1.0, -1.0), vec2(1.0, 1.0), vec2(-1.0, 1.0));
    var clip = u.view_proj * vec4(center, 1.0);
    clip = vec4(clip.xy + corners[vi] * u.point_size.xy * clip.w, clip.zw);
    var out: VsOut;
    out.pos = clip;
    out.color = color;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color;
}
"#;

pub struct CloudRenderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniform_buf: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    points: Option<(wgpu::Buffer, u32)>,
    target: Option<Target>,
}

struct Target {
    size: (u32, u32),
    color: wgpu::Texture,
    depth_view: wgpu::TextureView,
}

impl CloudRenderer {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("cloud shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &bgl,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buf.as_entire_binding() }],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&bgl)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("cloud pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Point>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Unorm8x4],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(COLOR_FORMAT.into())],
            }),
            multiview_mask: None,
            cache: None,
        });
        Self {
            device: device.clone(),
            queue: queue.clone(),
            pipeline,
            uniform_buf,
            bind_group,
            points: None,
            target: None,
        }
    }

    pub fn set_points(&mut self, count: u32) {
        let data = generate_cloud(count);
        let buf = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("points"),
            size: (data.len() * std::mem::size_of::<Point>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        self.queue.write_buffer(&buf, 0, bytemuck::cast_slice(&data));
        self.points = Some((buf, count));
    }

    /// Renders one frame and returns the texture to hand to Slint.
    pub fn render(&mut self, width: u32, height: u32, yaw: f32, pitch: f32, zoom: f32) -> wgpu::Texture {
        let (width, height) = (width.max(1), height.max(1));
        if self.target.as_ref().map(|t| t.size) != Some((width, height)) {
            self.target = Some(self.create_target(width, height));
        }
        let target = self.target.as_ref().unwrap();

        let aspect = width as f32 / height as f32;
        let eye = Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        ) * (3.0 / zoom);
        let view = Mat4::look_at_rh(eye, Vec3::ZERO, Vec3::Y);
        let proj = Mat4::perspective_rh(45f32.to_radians(), aspect, 0.05, 100.0);
        // ~1.5 physical pixels per point.
        let px = 1.5;
        let uniforms = Uniforms {
            view_proj: (proj * view).to_cols_array_2d(),
            point_size: [px / width as f32, px / height as f32, 0.0, 0.0],
        };
        self.queue.write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let color_view = target.color.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cloud pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &color_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.02, g: 0.02, b: 0.04, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                ..Default::default()
            });
            if let Some((buf, count)) = &self.points {
                pass.set_pipeline(&self.pipeline);
                pass.set_bind_group(0, &self.bind_group, &[]);
                pass.set_vertex_buffer(0, buf.slice(..));
                pass.draw(0..6, 0..*count);
            }
        }
        self.queue.submit(Some(encoder.finish()));
        target.color.clone()
    }

    fn create_target(&self, width: u32, height: u32) -> Target {
        let size = wgpu::Extent3d { width, height, depth_or_array_layers: 1 };
        let color = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cloud color"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: COLOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let depth = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("cloud depth"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        Target { size: (width, height), color, depth_view: depth.create_view(&Default::default()) }
    }
}

/// A synthetic "scan": a wavy ground plane plus a torus-knot object, colored by height.
fn generate_cloud(count: u32) -> Vec<Point> {
    let mut seed: u64 = 0x9E3779B97F4A7C15;
    let mut rnd = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 40) as f32 / (1u64 << 24) as f32
    };
    (0..count)
        .map(|i| {
            let pos = if i % 3 == 0 {
                let t = rnd() * std::f32::consts::TAU;
                let (p, q) = (2.0, 3.0);
                let r = 0.6 + 0.25 * (q * t).cos();
                let tube = 0.08 * rnd().sqrt();
                let a = rnd() * std::f32::consts::TAU;
                Vec3::new(
                    r * (p * t).cos() + tube * a.cos(),
                    0.25 * (q * t).sin() + tube * a.sin() + 0.2,
                    r * (p * t).sin() + tube * a.cos(),
                )
            } else {
                let x = rnd() * 2.0 - 1.0;
                let z = rnd() * 2.0 - 1.0;
                let y = -0.5 + 0.06 * (x * 6.0).sin() * (z * 5.0).cos() + 0.01 * rnd();
                Vec3::new(x * 1.5, y, z * 1.5)
            };
            let h = ((pos.y + 0.6) / 1.2).clamp(0.0, 1.0);
            let (r, g, b) = (h, 1.0 - (h - 0.5).abs() * 2.0, 1.0 - h);
            let color = (r * 255.0) as u32 | ((g * 255.0) as u32) << 8 | ((b * 255.0) as u32) << 16 | 0xFF << 24;
            Point { pos: pos.to_array(), color }
        })
        .collect()
}

use std::{
    marker::PhantomData,
    num::NonZeroU32,
    time::{Duration, Instant},
};

use criterion::{criterion_group, criterion_main, Criterion};

const DUMMY_TEXTURE_DESC: wgpu::TextureDescriptor = wgpu::TextureDescriptor {
    label: None,
    size: wgpu::Extent3d {
        width: 1,
        height: 1,
        depth_or_array_layers: 1,
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8UnormSrgb,
    usage: wgpu::TextureUsages::from_bits_truncate(
        wgpu::TextureUsages::RENDER_ATTACHMENT.bits() | wgpu::TextureUsages::TEXTURE_BINDING.bits(),
    ),
};

struct WgpuBench {
    device: wgpu::Device,
    queue: wgpu::Queue,
    dummy_attachment: wgpu::TextureView,
}
impl WgpuBench {
    fn new() -> Self {
        let backends = wgpu::util::backend_bits_from_env().unwrap_or(wgpu::Backends::all());
        let instance = wgpu::Instance::new(backends);

        let adapter = pollster::block_on(wgpu::util::initialize_adapter_from_env_or_default(
            &instance, backends, None,
        ))
        .unwrap();
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                features: adapter.features(),
                limits: adapter.limits(),
            },
            None,
        ))
        .unwrap();

        let dummy_attachment = device
            .create_texture(&DUMMY_TEXTURE_DESC)
            .create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            device,
            queue,
            dummy_attachment,
        }
    }

    fn rpass_attachments(&self) -> [wgpu::RenderPassColorAttachment<'_>; 1] {
        [wgpu::RenderPassColorAttachment {
            view: &self.dummy_attachment,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: false,
            },
        }]
    }

    fn rpass_desc<'a>(
        &self,
        color_attachments: &'a [wgpu::RenderPassColorAttachment<'_>],
    ) -> wgpu::RenderPassDescriptor<'a, '_> {
        wgpu::RenderPassDescriptor {
            label: None,
            color_attachments,
            depth_stencil_attachment: None,
        }
    }
}

#[derive(Default)]
struct LifetimeBound<'a: 'b, 'b>(PhantomData<(&'a (), &'b ())>);

fn wgpu_renderpass_bench<'a>(
    c: &mut Criterion,
    wb: &WgpuBench,
    name: &str,
    mut recording_fn: impl for<'b> FnMut(&mut wgpu::RenderPass<'b>, LifetimeBound<'a, 'b>) + 'a,
) {
    let ca = wb.rpass_attachments();
    let rpass_desc = wb.rpass_desc(&ca);

    c.benchmark_group("rpass")
        .measurement_time(Duration::from_secs(20))
        .bench_function(name, |b| {
            b.iter_custom(|iterations| {
                let mut encoder = wb
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                let mut duration = Duration::from_secs(0);
                for _ in 0..iterations {
                    let mut rpass = encoder.begin_render_pass(&rpass_desc);
                    recording_fn(&mut rpass, LifetimeBound::default());
                    let start = Instant::now();
                    drop(rpass);
                    duration += start.elapsed();
                }
                wb.queue.submit(Some(encoder.finish()));
                duration
            })
        });
}

const NUMBER_OF_RESOURCES: u32 = 10_000;

fn empty_rpass(c: &mut Criterion) {
    let wb = WgpuBench::new();

    wgpu_renderpass_bench(c, &wb, "empty renderpass", |_, _| {});
}

fn single_bg(c: &mut Criterion) {
    let wb = WgpuBench::new();

    let textures: Vec<_> = (0..NUMBER_OF_RESOURCES)
        .map(|_| {
            wb.device
                .create_texture(&DUMMY_TEXTURE_DESC)
                .create_view(&wgpu::TextureViewDescriptor::default())
        })
        .collect();

    let texture_refs: Vec<_> = textures.iter().collect();

    let bind_group_layout = wb
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: NonZeroU32::new(NUMBER_OF_RESOURCES),
            }],
        });

    let bind_group = wb.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureViewArray(&texture_refs),
        }],
    });

    wgpu_renderpass_bench(c, &wb, "single bind group", |rpass, _| {
        rpass.set_bind_group(0, &bind_group, &[])
    });

    drop(bind_group);
    drop(bind_group_layout);
    drop(texture_refs);
    drop(textures);

    wb.device.poll(wgpu::Maintain::Wait);
}

fn many_bg(c: &mut Criterion) {
    let wb = WgpuBench::new();

    let textures: Vec<_> = (0..NUMBER_OF_RESOURCES)
        .map(|_| {
            wb.device
                .create_texture(&DUMMY_TEXTURE_DESC)
                .create_view(&wgpu::TextureViewDescriptor::default())
        })
        .collect();

    let bind_group_layout = wb
        .device
        .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });

    let bind_groups: Vec<_> = textures
        .iter()
        .map(|texture| {
            wb.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture),
                }],
            })
        })
        .collect();

    wgpu_renderpass_bench(c, &wb, "multiple bind groups", |rpass, _| {
        for bind_group in &bind_groups {
            rpass.set_bind_group(0, bind_group, &[])
        }
    });

    drop(bind_groups);
    drop(bind_group_layout);
    drop(textures);

    wb.device.poll(wgpu::Maintain::Wait);
}

criterion_group!(benches, empty_rpass, single_bg, many_bg);
criterion_main!(benches);
